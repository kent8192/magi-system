//! Codex app-server bridge for live magi delivery.
//!
//! The bridge tails Redis Pub/Sub for the active magi session agent. Each
//! delivered inbox batch is converted into compact context lines and submitted
//! to the Codex app-server through `codex app-server proxy`. A new Codex turn is
//! started for normal idle sessions; if the app-server rejects the turn, the
//! bridge falls back to `thread/inject_items` so the message is still persisted
//! into model-visible thread history.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{SystemTime, UNIX_EPOCH};

use futures_util::StreamExt;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};

use crate::config::AppConfig;
use crate::error::{MagiError, Result};
use crate::messaging::{self, InboxReadMode, MessageRecord};
use crate::model::RedisKeys;
use crate::session_identity::{missing_session_agent_message, resolve_identity};

/// Runs the long-lived Codex app-server bridge.
pub async fn run(thread: Option<String>, cwd: Option<PathBuf>, codex: String) -> Result<()> {
    let thread_id = resolve_thread_id(thread)?;
    let config = AppConfig::load()?;
    let url = config
        .redis
        .url
        .clone()
        .ok_or_else(|| MagiError::InvalidConfig("redis.url is not configured".to_string()))?;
    let identity = resolve_identity(&config);
    let team = identity
        .team
        .ok_or_else(|| MagiError::InvalidConfig("identity.active_team is required".to_string()))?;
    let agent = identity
        .agent
        .ok_or_else(|| MagiError::InvalidConfig(missing_session_agent_message()))?;
    let cwd = cwd
        .or_else(|| std::env::current_dir().ok())
        .map(|path| path.to_string_lossy().to_string());

    let client = redis::Client::open(url.as_str())?;
    let mut pubsub = client.get_async_pubsub().await?;
    pubsub.subscribe(RedisKeys::new(&team).pubsub()).await?;
    let mut wakeups = pubsub.on_message();
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));
    let status_path = bridge_status_path(&thread_id);

    write_bridge_status(status_path.as_deref(), "running", None);
    deliver_pending_and_record(
        &url,
        &team,
        &agent,
        &thread_id,
        cwd.as_deref(),
        &codex,
        status_path.as_deref(),
    )
    .await;

    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                write_bridge_status(status_path.as_deref(), "stopped", None);
                return Ok(());
            },
            _ = interval.tick() => {
                deliver_pending_and_record(
                    &url,
                    &team,
                    &agent,
                    &thread_id,
                    cwd.as_deref(),
                    &codex,
                    status_path.as_deref(),
                ).await;
            }
            Some(_) = wakeups.next() => {
                deliver_pending_and_record(
                    &url,
                    &team,
                    &agent,
                    &thread_id,
                    cwd.as_deref(),
                    &codex,
                    status_path.as_deref(),
                ).await;
            }
        }
    }
}

fn resolve_thread_id(thread: Option<String>) -> Result<String> {
    thread
        .or_else(|| std::env::var("CODEX_THREAD_ID").ok())
        .or_else(|| std::env::var("CODEX_SESSION_ID").ok())
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            MagiError::InvalidConfig(
                "`magi codex bridge` requires --thread, CODEX_THREAD_ID, or CODEX_SESSION_ID"
                    .to_string(),
            )
        })
}

async fn deliver_pending(
    url: &str,
    team: &str,
    agent: &str,
    thread_id: &str,
    cwd: Option<&str>,
    codex: &str,
) -> Result<()> {
    let messages = messaging::read_inbox_with_url(url, team, agent, InboxReadMode::Peek).await?;
    if messages.is_empty() {
        return Ok(());
    }

    let last_delivered_id = messages
        .last()
        .map(|message| message.id.clone())
        .unwrap_or_default();
    for message in messages {
        let text = format_context_line(&message);
        submit_to_codex(codex, thread_id, cwd, &message.id, &text).await?;
    }
    messaging::advance_inbox_cursor_with_url(url, team, agent, &last_delivered_id).await?;
    Ok(())
}

async fn deliver_pending_and_record(
    url: &str,
    team: &str,
    agent: &str,
    thread_id: &str,
    cwd: Option<&str>,
    codex: &str,
    status_path: Option<&Path>,
) {
    match deliver_pending(url, team, agent, thread_id, cwd, codex).await {
        Ok(()) => write_bridge_status(status_path, "running", None),
        Err(error) => {
            let state = bridge_status_state_for_error(&error);
            let status_error = bridge_status_error_message(&error);
            eprintln!("magi codex bridge delivery failed; will retry: {status_error}");
            write_bridge_status(status_path, state, Some(&status_error));
        }
    }
}

/// Returns the bridge status state that should be recorded for a delivery error.
pub fn bridge_status_state_for_error(_error: &MagiError) -> &'static str {
    "retrying"
}

/// Formats a delivery error for hook-visible bridge status.
pub fn bridge_status_error_message(error: &MagiError) -> String {
    let message = error.to_string();
    if message.contains("codex app-server proxy closed before JSON-RPC response") {
        format!(
            "{message}; check codex app-server control socket at {}",
            default_codex_app_server_socket().display()
        )
    } else {
        message
    }
}

fn default_codex_app_server_socket() -> PathBuf {
    let codex_home = std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".codex")))
        .unwrap_or_else(|| PathBuf::from(".codex"));
    codex_home.join("app-server-control/app-server-control.sock")
}

fn bridge_status_path(thread_id: &str) -> Option<PathBuf> {
    let state_dir = std::env::var_os("MAGI_CODEX_STATE_DIR")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("XDG_STATE_HOME").map(|home| PathBuf::from(home).join("magi-codex"))
        })
        .or_else(|| {
            std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state/magi-codex"))
        })?;
    Some(
        state_dir
            .join("bridges")
            .join(format!("{}.status", safe_key(thread_id))),
    )
}

fn safe_key(value: &str) -> String {
    value
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        .collect()
}

fn write_bridge_status(path: Option<&Path>, state: &str, last_error: Option<&str>) {
    let Some(path) = path else {
        return;
    };
    let Some(parent) = path.parent() else {
        return;
    };
    if let Err(error) = std::fs::create_dir_all(parent) {
        eprintln!("magi codex bridge could not create status directory: {error}");
        return;
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default();
    let pid = std::process::id();
    let last_error = sanitize_status_value(last_error.unwrap_or(""));
    let content = format!("state={state}\npid={pid}\nupdated_at={now}\nlast_error={last_error}\n");
    if let Err(error) = std::fs::write(path, content) {
        eprintln!("magi codex bridge could not write status: {error}");
    }
}

fn sanitize_status_value(value: &str) -> String {
    value
        .chars()
        .filter(|c| !matches!(c, '\n' | '\r'))
        .collect()
}

/// Formats a magi message as the context line injected into Codex.
pub fn format_context_line(message: &MessageRecord) -> String {
    format!(
        "{}->{}: {}",
        message.event.from, message.event.to, message.event.body
    )
}

/// Submits a context line to the Codex app-server.
pub async fn submit_to_codex(
    codex: &str,
    thread_id: &str,
    cwd: Option<&str>,
    message_id: &str,
    text: &str,
) -> Result<()> {
    let mut child = spawn_codex_proxy(codex)?;
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| MagiError::CommandFailed("codex proxy stdin unavailable".to_string()))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| MagiError::CommandFailed("codex proxy stdout unavailable".to_string()))?;

    let mut client = JsonRpcClient {
        stdin,
        stdout: BufReader::new(stdout),
        next_id: 1,
    };

    client
        .request(json!({
            "method": "initialize",
            "params": {
                "clientInfo": {
                    "name": "magi-codex-bridge",
                    "title": "magi Codex Bridge",
                    "version": env!("CARGO_PKG_VERSION")
                }
            }
        }))
        .await?;
    client
        .request(json!({
            "method": "thread/resume",
            "params": {
                "threadId": thread_id,
                "excludeTurns": true
            }
        }))
        .await?;

    let turn = build_turn_start_request(thread_id, cwd, message_id, text);
    if let Err(turn_error) = client.request(turn).await {
        let inject = build_inject_items_request(thread_id, message_id, text);
        client.request(inject).await.map_err(|inject_error| {
            MagiError::CommandFailed(format!(
                "codex app-server turn/start failed ({turn_error}); fallback thread/inject_items failed ({inject_error})"
            ))
        })?;
    }

    drop(client);
    let status = child.wait().await?;
    if !status.success() {
        return Err(MagiError::CommandFailed(format!(
            "codex app-server proxy exited with {status}"
        )));
    }
    Ok(())
}

fn spawn_codex_proxy(codex: &str) -> Result<Child> {
    Command::new(codex)
        .arg("app-server")
        .arg("proxy")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(MagiError::Io)
}

/// Builds the JSON-RPC envelope for starting a Codex turn.
pub fn build_turn_start_request(
    thread_id: &str,
    cwd: Option<&str>,
    message_id: &str,
    text: &str,
) -> Value {
    let mut params = json!({
        "threadId": thread_id,
        "clientUserMessageId": format!("magi-{message_id}"),
        "input": [{ "type": "text", "text": text }]
    });
    if let Some(cwd) = cwd.filter(|cwd| !cwd.is_empty()) {
        params["cwd"] = json!(cwd);
    }
    json!({
        "method": "turn/start",
        "params": params
    })
}

/// Builds the JSON-RPC envelope for persisting a fallback user message.
pub fn build_inject_items_request(thread_id: &str, message_id: &str, text: &str) -> Value {
    json!({
        "method": "thread/inject_items",
        "params": {
            "threadId": thread_id,
            "items": [{
                "type": "message",
                "role": "user",
                "content": [{ "type": "input_text", "text": text }],
                "clientId": format!("magi-{message_id}")
            }]
        }
    })
}

struct JsonRpcClient {
    stdin: tokio::process::ChildStdin,
    stdout: BufReader<tokio::process::ChildStdout>,
    next_id: u64,
}

impl JsonRpcClient {
    async fn request(&mut self, mut request: Value) -> Result<Value> {
        let id = self.next_id;
        self.next_id += 1;
        request["id"] = json!(id);
        let line = serde_json::to_string(&request).map_err(|error| {
            MagiError::CommandFailed(format!("failed to encode codex JSON-RPC request: {error}"))
        })?;
        self.stdin.write_all(line.as_bytes()).await?;
        self.stdin.write_all(b"\n").await?;
        self.stdin.flush().await?;

        loop {
            let mut response = String::new();
            let read = self.stdout.read_line(&mut response).await?;
            if read == 0 {
                return Err(MagiError::CommandFailed(
                    "codex app-server proxy closed before JSON-RPC response".to_string(),
                ));
            }
            let value: Value = serde_json::from_str(response.trim()).map_err(|error| {
                MagiError::CommandFailed(format!(
                    "failed to decode codex JSON-RPC response `{}`: {error}",
                    response.trim()
                ))
            })?;
            if value.get("id").and_then(Value::as_u64) != Some(id) {
                continue;
            }
            if let Some(error) = value.get("error") {
                return Err(MagiError::CommandFailed(format!(
                    "codex app-server request `{}` failed: {error}",
                    request
                        .get("method")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown")
                )));
            }
            return Ok(value.get("result").cloned().unwrap_or_else(|| json!({})));
        }
    }
}
