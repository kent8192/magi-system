//! Codex app-server bridge for live magi delivery.
//!
//! The bridge tails Redis Pub/Sub for the active magi session agent. Each
//! delivered inbox batch is converted into compact context lines and submitted
//! to the Codex app-server over the Unix control socket's WebSocket transport. A
//! new Codex turn is started for normal idle sessions; if the app-server rejects
//! the turn, the bridge falls back to `thread/inject_items` so the message is
//! still persisted into model-visible thread history.

use std::future::Future;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::prelude::*;
use futures_util::StreamExt;
use serde_json::{json, Value};
use sha1::{Digest, Sha1};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

use crate::config::AppConfig;
use crate::error::{MagiError, Result};
use crate::messaging::{self, InboxReadMode, MessageRecord};
use crate::model::RedisKeys;
use crate::session_identity::{missing_session_agent_message, resolve_identity};

#[cfg(not(test))]
const CODEX_APP_SERVER_RPC_TIMEOUT: Duration = Duration::from_secs(10);
#[cfg(test)]
const CODEX_APP_SERVER_RPC_TIMEOUT: Duration = Duration::from_millis(100);
const MAX_WEBSOCKET_PAYLOAD_BYTES: usize = 16 * 1024 * 1024;
const WEBSOCKET_GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

/// Runs the long-lived Codex app-server bridge.
pub async fn run(
    thread: Option<String>,
    cwd: Option<PathBuf>,
    codex: String,
    socket: Option<PathBuf>,
) -> Result<()> {
    let thread_id = resolve_thread_id(thread)?;
    let socket = resolve_codex_app_server_socket(socket);
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
    let delivery = BridgeDelivery {
        url: &url,
        team: &team,
        agent: &agent,
        thread_id: &thread_id,
        cwd: cwd.as_deref(),
        codex: &codex,
        socket: &socket,
    };

    write_bridge_status(status_path.as_deref(), "running", None);
    deliver_pending_and_record(&delivery, status_path.as_deref()).await;

    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                write_bridge_status(status_path.as_deref(), "stopped", None);
                return Ok(());
            },
            _ = interval.tick() => {
                deliver_pending_and_record(&delivery, status_path.as_deref()).await;
            }
            Some(_) = wakeups.next() => {
                deliver_pending_and_record(&delivery, status_path.as_deref()).await;
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeliveryOutcome {
    NoPending,
    Delivered,
}

#[derive(Debug)]
struct DeliveryProgress {
    outcome: DeliveryOutcome,
    last_delivered_id: Option<String>,
}

#[derive(Debug)]
struct DeliveryFailure {
    error: MagiError,
    last_delivered_id: Option<String>,
}

struct BridgeDelivery<'a> {
    url: &'a str,
    team: &'a str,
    agent: &'a str,
    thread_id: &'a str,
    cwd: Option<&'a str>,
    codex: &'a str,
    socket: &'a Path,
}

async fn deliver_pending(
    delivery: &BridgeDelivery<'_>,
    status_path: Option<&Path>,
) -> Result<DeliveryOutcome> {
    ensure_codex_app_server_socket(delivery.socket)?;
    crate::actas::ensure_unblocked_for_session(delivery.url, delivery.team, delivery.agent).await?;
    let messages = messaging::read_inbox_with_url(
        delivery.url,
        delivery.team,
        delivery.agent,
        InboxReadMode::Peek,
    )
    .await?;
    if !messages.is_empty() {
        write_bridge_status(status_path, "delivering", None);
    }
    let progress = deliver_messages_with_submitter(
        messages,
        delivery.thread_id,
        delivery.cwd,
        |thread_id, cwd, message_id, text| {
            let codex = delivery.codex.to_string();
            let socket = delivery.socket.to_path_buf();
            async move {
                submit_to_codex_with_socket(
                    &codex,
                    &socket,
                    &thread_id,
                    cwd.as_deref(),
                    &message_id,
                    &text,
                )
                .await
            }
        },
    )
    .await;

    match progress {
        Ok(progress) => {
            if let Some(last_delivered_id) = progress.last_delivered_id {
                messaging::advance_inbox_cursor_with_url(
                    delivery.url,
                    delivery.team,
                    delivery.agent,
                    &last_delivered_id,
                )
                .await?;
            }
            Ok(progress.outcome)
        }
        Err(failure) => {
            if let Some(last_delivered_id) = failure.last_delivered_id {
                messaging::advance_inbox_cursor_with_url(
                    delivery.url,
                    delivery.team,
                    delivery.agent,
                    &last_delivered_id,
                )
                .await?;
            }
            Err(failure.error)
        }
    }
}

async fn deliver_messages_with_submitter<F, Fut>(
    messages: Vec<MessageRecord>,
    thread_id: &str,
    cwd: Option<&str>,
    mut submit: F,
) -> std::result::Result<DeliveryProgress, DeliveryFailure>
where
    F: FnMut(String, Option<String>, String, String) -> Fut,
    Fut: Future<Output = Result<()>>,
{
    if messages.is_empty() {
        return Ok(DeliveryProgress {
            outcome: DeliveryOutcome::NoPending,
            last_delivered_id: None,
        });
    }

    let thread_id = thread_id.to_string();
    let cwd = cwd.map(ToOwned::to_owned);
    let mut last_delivered_id = None;
    for message in messages {
        let message_id = message.id.clone();
        let text = format_context_line(&message);
        if let Err(error) = submit(thread_id.clone(), cwd.clone(), message_id.clone(), text).await {
            return Err(DeliveryFailure {
                error,
                last_delivered_id,
            });
        }
        last_delivered_id = Some(message_id);
    }

    Ok(DeliveryProgress {
        outcome: DeliveryOutcome::Delivered,
        last_delivered_id,
    })
}

async fn deliver_pending_and_record(delivery: &BridgeDelivery<'_>, status_path: Option<&Path>) {
    match deliver_pending(delivery, status_path).await {
        Ok(outcome) => record_delivery_outcome(status_path, outcome),
        Err(error) => {
            let state = bridge_status_state_for_error(&error);
            let status_error = bridge_status_error_message(&error);
            eprintln!("magi codex bridge delivery failed; state={state}: {status_error}");
            write_bridge_status(status_path, state, Some(&status_error));
        }
    }
}

fn record_delivery_outcome(status_path: Option<&Path>, outcome: DeliveryOutcome) {
    match outcome {
        DeliveryOutcome::Delivered => write_bridge_status(status_path, "running", None),
        DeliveryOutcome::NoPending => {
            if matches!(
                current_bridge_status_state(status_path).as_deref(),
                None | Some("starting") | Some("delivering")
            ) {
                write_bridge_status(status_path, "running", None);
            }
        }
    }
}

/// Returns the bridge status state that should be recorded for a delivery error.
pub fn bridge_status_state_for_error(error: &MagiError) -> &'static str {
    match error {
        MagiError::UnsupportedRuntime(_) => "unsupported",
        _ => "retrying",
    }
}

/// Formats a delivery error for hook-visible bridge status.
pub fn bridge_status_error_message(error: &MagiError) -> String {
    if let MagiError::UnsupportedRuntime(message) = error {
        return message.clone();
    }

    let message = error.to_string();
    if message.contains("codex app-server websocket closed before JSON-RPC response") {
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

/// Resolves the Codex app-server Unix control socket used by the bridge.
pub fn resolve_codex_app_server_socket(explicit: Option<PathBuf>) -> PathBuf {
    explicit
        .or_else(|| std::env::var_os("MAGI_CODEX_APP_SERVER_SOCKET").map(PathBuf::from))
        .unwrap_or_else(default_codex_app_server_socket)
}

fn ensure_codex_app_server_socket(socket: &Path) -> Result<()> {
    let metadata = match std::fs::metadata(socket) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(missing_codex_socket_error(socket));
        }
        Err(error) => return Err(MagiError::Io(error)),
    };

    #[cfg(unix)]
    {
        use std::os::unix::fs::FileTypeExt;

        if !metadata.file_type().is_socket() {
            return Err(MagiError::UnsupportedRuntime(format!(
                "Codex app-server control socket path is not a Unix socket at {}; set MAGI_CODEX_APP_SERVER_SOCKET to a reachable Unix socket or start a managed Codex app-server daemon. stdio:// app-server processes cannot be reached by magi codex bridge.",
                socket.display()
            )));
        }
    }

    Ok(())
}

fn missing_codex_socket_error(socket: &Path) -> MagiError {
    MagiError::UnsupportedRuntime(format!(
        "Codex app-server control socket not found at {}; set MAGI_CODEX_APP_SERVER_SOCKET to a reachable Unix socket or start a managed Codex app-server daemon. stdio:// app-server processes cannot be reached by magi codex bridge.",
        socket.display()
    ))
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

fn current_bridge_status_state(path: Option<&Path>) -> Option<String> {
    let path = path?;
    let content = std::fs::read_to_string(path).ok()?;
    content
        .lines()
        .find_map(|line| line.strip_prefix("state=").map(ToOwned::to_owned))
        .filter(|state| !state.is_empty())
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
    let socket = resolve_codex_app_server_socket(None);
    submit_to_codex_with_socket(codex, &socket, thread_id, cwd, message_id, text).await
}

/// Submits a context line to the Codex app-server through a specific socket.
pub async fn submit_to_codex_with_socket(
    _codex: &str,
    socket: &Path,
    thread_id: &str,
    cwd: Option<&str>,
    message_id: &str,
    text: &str,
) -> Result<()> {
    let mut client = WebSocketJsonRpcClient::connect(socket).await?;

    client
        .request(json!({
            "method": "initialize",
            "params": {
                "capabilities": null,
                "clientInfo": {
                    "name": "magi-codex-bridge",
                    "title": "magi Codex Bridge",
                    "version": env!("CARGO_PKG_VERSION")
                }
            }
        }))
        .await?;
    client
        .notification(json!({
            "method": "initialized"
        }))
        .await?;
    client
        .request(json!({
            "method": "thread/resume",
            "params": {
                "threadId": thread_id
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

    Ok(())
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

struct WebSocketJsonRpcClient {
    stream: UnixStream,
    next_id: u64,
}

impl WebSocketJsonRpcClient {
    async fn connect(socket: &Path) -> Result<Self> {
        let mut stream = timeout_command(
            "codex app-server websocket connect",
            UnixStream::connect(socket),
        )
        .await?;
        websocket_handshake(&mut stream).await?;
        Ok(Self { stream, next_id: 1 })
    }

    async fn request(&mut self, mut request: Value) -> Result<Value> {
        let id = self.next_id;
        self.next_id += 1;
        request["id"] = json!(id);
        let method = request
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string();
        timeout_command(
            &format!("codex app-server request `{method}`"),
            self.request_inner(request, id, &method),
        )
        .await
    }

    async fn notification(&mut self, notification: Value) -> Result<()> {
        let method = notification
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string();
        timeout_command(
            &format!("codex app-server notification `{method}`"),
            self.send_json(&notification),
        )
        .await
    }

    async fn request_inner(&mut self, request: Value, id: u64, method: &str) -> Result<Value> {
        self.send_json(&request).await?;
        loop {
            let message = self.read_json_message().await?;
            if message.get("id").and_then(Value::as_u64) != Some(id) {
                continue;
            }
            if let Some(error) = message.get("error") {
                return Err(MagiError::CommandFailed(format!(
                    "codex app-server request `{method}` failed: {error}"
                )));
            }
            return Ok(message.get("result").cloned().unwrap_or_else(|| json!({})));
        }
    }

    async fn send_json(&mut self, value: &Value) -> Result<()> {
        let payload = serde_json::to_vec(value).map_err(|error| {
            MagiError::CommandFailed(format!("failed to encode codex JSON-RPC message: {error}"))
        })?;
        write_websocket_frame(&mut self.stream, WebSocketOpcode::Text, &payload).await
    }

    async fn read_json_message(&mut self) -> Result<Value> {
        loop {
            let frame = read_websocket_frame(&mut self.stream).await?;
            match frame.opcode {
                WebSocketOpcode::Text => {
                    return serde_json::from_slice(&frame.payload).map_err(|error| {
                        MagiError::CommandFailed(format!(
                            "failed to decode codex JSON-RPC websocket message: {error}"
                        ))
                    });
                }
                WebSocketOpcode::Ping => {
                    write_websocket_frame(&mut self.stream, WebSocketOpcode::Pong, &frame.payload)
                        .await?;
                }
                WebSocketOpcode::Pong => {}
                WebSocketOpcode::Close => {
                    return Err(MagiError::CommandFailed(
                        "codex app-server websocket closed before JSON-RPC response".to_string(),
                    ));
                }
                WebSocketOpcode::Binary | WebSocketOpcode::Continuation => {
                    return Err(MagiError::CommandFailed(format!(
                        "codex app-server sent unsupported websocket opcode {}",
                        frame.opcode.code()
                    )));
                }
            }
        }
    }
}

async fn timeout_command<T, E, Fut>(operation: &str, future: Fut) -> Result<T>
where
    E: Into<MagiError>,
    Fut: Future<Output = std::result::Result<T, E>>,
{
    match tokio::time::timeout(CODEX_APP_SERVER_RPC_TIMEOUT, future).await {
        Ok(result) => result.map_err(Into::into),
        Err(_) => Err(MagiError::CommandFailed(format!(
            "{operation} timed out after {}s",
            CODEX_APP_SERVER_RPC_TIMEOUT.as_secs()
        ))),
    }
}

async fn websocket_handshake(stream: &mut UnixStream) -> Result<()> {
    timeout_command("codex app-server websocket handshake", async {
        let key = websocket_key();
        let request = format!(
            "GET / HTTP/1.1\r\n\
             Host: localhost\r\n\
             Upgrade: websocket\r\n\
             Connection: Upgrade\r\n\
             Sec-WebSocket-Version: 13\r\n\
             Sec-WebSocket-Key: {key}\r\n\
             \r\n"
        );
        stream.write_all(request.as_bytes()).await?;
        stream.flush().await?;

        let response = read_http_upgrade_response(stream).await?;
        validate_websocket_upgrade_response(&response, &key)
    })
    .await
}

fn websocket_key() -> String {
    let nonce: [u8; 16] = rand::random();
    BASE64_STANDARD.encode(nonce)
}

async fn read_http_upgrade_response(stream: &mut UnixStream) -> Result<String> {
    let mut response = Vec::new();
    let mut byte = [0u8; 1];
    while response.len() < 16 * 1024 {
        let read = stream.read(&mut byte).await?;
        if read == 0 {
            return Err(MagiError::CommandFailed(
                "codex app-server closed during websocket handshake".to_string(),
            ));
        }
        response.push(byte[0]);
        if response.ends_with(b"\r\n\r\n") {
            return String::from_utf8(response).map_err(|error| {
                MagiError::CommandFailed(format!(
                    "codex app-server sent non-UTF-8 websocket handshake response: {error}"
                ))
            });
        }
    }
    Err(MagiError::CommandFailed(
        "codex app-server websocket handshake response exceeded 16 KiB".to_string(),
    ))
}

fn validate_websocket_upgrade_response(response: &str, key: &str) -> Result<()> {
    let mut lines = response.split("\r\n");
    let status = lines.next().unwrap_or_default();
    if !status.contains(" 101 ") {
        return Err(MagiError::CommandFailed(format!(
            "codex app-server websocket handshake failed: {status}"
        )));
    }

    let upgrade = http_header(response, "upgrade")
        .map(|value| value.eq_ignore_ascii_case("websocket"))
        .unwrap_or(false);
    let connection = http_header(response, "connection")
        .map(|value| {
            value
                .split(',')
                .any(|part| part.trim().eq_ignore_ascii_case("upgrade"))
        })
        .unwrap_or(false);
    let accept = http_header(response, "sec-websocket-accept")
        .map(|value| value == websocket_accept(key))
        .unwrap_or(false);

    if !(upgrade && connection && accept) {
        return Err(MagiError::CommandFailed(
            "codex app-server websocket handshake response was missing required upgrade headers"
                .to_string(),
        ));
    }
    Ok(())
}

fn http_header<'a>(response: &'a str, name: &str) -> Option<&'a str> {
    response.lines().skip(1).find_map(|line| {
        let (header_name, value) = line.split_once(':')?;
        header_name
            .trim()
            .eq_ignore_ascii_case(name)
            .then(|| value.trim())
    })
}

fn websocket_accept(key: &str) -> String {
    let mut hasher = Sha1::new();
    hasher.update(key.as_bytes());
    hasher.update(WEBSOCKET_GUID.as_bytes());
    BASE64_STANDARD.encode(hasher.finalize())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WebSocketOpcode {
    Continuation,
    Text,
    Binary,
    Close,
    Ping,
    Pong,
}

impl WebSocketOpcode {
    fn from_code(code: u8) -> Result<Self> {
        match code {
            0x0 => Ok(Self::Continuation),
            0x1 => Ok(Self::Text),
            0x2 => Ok(Self::Binary),
            0x8 => Ok(Self::Close),
            0x9 => Ok(Self::Ping),
            0xA => Ok(Self::Pong),
            _ => Err(MagiError::CommandFailed(format!(
                "codex app-server sent unsupported websocket opcode {code}"
            ))),
        }
    }

    fn code(self) -> u8 {
        match self {
            Self::Continuation => 0x0,
            Self::Text => 0x1,
            Self::Binary => 0x2,
            Self::Close => 0x8,
            Self::Ping => 0x9,
            Self::Pong => 0xA,
        }
    }
}

struct WebSocketFrame {
    opcode: WebSocketOpcode,
    payload: Vec<u8>,
}

async fn write_websocket_frame(
    stream: &mut UnixStream,
    opcode: WebSocketOpcode,
    payload: &[u8],
) -> Result<()> {
    let mut header = Vec::with_capacity(14);
    header.push(0x80 | opcode.code());
    if payload.len() <= 125 {
        header.push(0x80 | payload.len() as u8);
    } else if payload.len() <= u16::MAX as usize {
        header.push(0x80 | 126);
        header.extend_from_slice(&(payload.len() as u16).to_be_bytes());
    } else {
        header.push(0x80 | 127);
        header.extend_from_slice(&(payload.len() as u64).to_be_bytes());
    }

    let mask: [u8; 4] = rand::random();
    header.extend_from_slice(&mask);
    let mut masked = Vec::with_capacity(payload.len());
    for (index, byte) in payload.iter().enumerate() {
        masked.push(byte ^ mask[index % 4]);
    }

    stream.write_all(&header).await?;
    stream.write_all(&masked).await?;
    stream.flush().await?;
    Ok(())
}

async fn read_websocket_frame(stream: &mut UnixStream) -> Result<WebSocketFrame> {
    let mut header = [0u8; 2];
    stream.read_exact(&mut header).await.map_err(|error| {
        if error.kind() == ErrorKind::UnexpectedEof {
            MagiError::CommandFailed(
                "codex app-server websocket closed before JSON-RPC response".to_string(),
            )
        } else {
            MagiError::Io(error)
        }
    })?;

    if header[0] & 0x80 == 0 {
        return Err(MagiError::CommandFailed(
            "codex app-server sent fragmented websocket frames, which are unsupported".to_string(),
        ));
    }

    let opcode = WebSocketOpcode::from_code(header[0] & 0x0F)?;
    let masked = header[1] & 0x80 != 0;
    let mut length = u64::from(header[1] & 0x7F);
    if length == 126 {
        let mut extended = [0u8; 2];
        stream.read_exact(&mut extended).await?;
        length = u64::from(u16::from_be_bytes(extended));
    } else if length == 127 {
        let mut extended = [0u8; 8];
        stream.read_exact(&mut extended).await?;
        length = u64::from_be_bytes(extended);
    }
    if length > MAX_WEBSOCKET_PAYLOAD_BYTES as u64 {
        return Err(MagiError::CommandFailed(format!(
            "codex app-server websocket frame exceeded {} bytes",
            MAX_WEBSOCKET_PAYLOAD_BYTES
        )));
    }

    let mask = if masked {
        let mut mask = [0u8; 4];
        stream.read_exact(&mut mask).await?;
        Some(mask)
    } else {
        None
    };
    let mut payload = vec![0u8; length as usize];
    stream.read_exact(&mut payload).await?;
    if let Some(mask) = mask {
        for (index, byte) in payload.iter_mut().enumerate() {
            *byte ^= mask[index % 4];
        }
    }
    Ok(WebSocketFrame { opcode, payload })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    fn message(id: &str, body: &str) -> MessageRecord {
        MessageRecord {
            id: id.to_string(),
            event: crate::model::MessageEvent {
                from: "sender".to_string(),
                to: "recipient".to_string(),
                body: body.to_string(),
                created_at: "123".to_string(),
            },
        }
    }

    #[test]
    fn no_pending_delivery_preserves_retrying_status() {
        let temp = tempfile::tempdir().expect("temp dir");
        let status_path = temp.path().join("bridge.status");
        write_bridge_status(
            Some(&status_path),
            "retrying",
            Some("failed to connect to app-server"),
        );

        record_delivery_outcome(Some(&status_path), DeliveryOutcome::NoPending);

        let status = std::fs::read_to_string(status_path).expect("status");
        assert!(status.contains("state=retrying"));
        assert!(status.contains("last_error=failed to connect to app-server"));
    }

    #[test]
    fn no_pending_delivery_clears_stale_delivering_status() {
        let temp = tempfile::tempdir().expect("temp dir");
        let status_path = temp.path().join("bridge.status");
        write_bridge_status(Some(&status_path), "delivering", None);

        record_delivery_outcome(Some(&status_path), DeliveryOutcome::NoPending);

        let status = std::fs::read_to_string(status_path).expect("status");
        assert!(status.contains("state=running"));
    }

    #[test]
    fn successful_delivery_clears_previous_retrying_status() {
        let temp = tempfile::tempdir().expect("temp dir");
        let status_path = temp.path().join("bridge.status");
        write_bridge_status(
            Some(&status_path),
            "retrying",
            Some("failed to connect to app-server"),
        );

        record_delivery_outcome(Some(&status_path), DeliveryOutcome::Delivered);

        let status = std::fs::read_to_string(status_path).expect("status");
        assert!(status.contains("state=running"));
        assert!(status.contains("last_error=\n"));
    }

    #[tokio::test]
    async fn app_server_rpc_timeout_is_reported_as_retrying_status() {
        let error = timeout_command(
            "codex app-server request `initialize`",
            std::future::pending::<Result<()>>(),
        )
        .await
        .expect_err("pending RPC should time out");

        assert_eq!(bridge_status_state_for_error(&error), "retrying");
        assert!(bridge_status_error_message(&error).contains("timed out"));
    }

    #[test]
    fn missing_codex_socket_preflight_reports_unsupported_runtime() {
        let temp = tempfile::tempdir().expect("temp dir");
        let missing_socket = temp.path().join("app-server.sock");

        let error = ensure_codex_app_server_socket(&missing_socket)
            .expect_err("missing socket should be unsupported");

        assert_eq!(bridge_status_state_for_error(&error), "unsupported");
        let message = bridge_status_error_message(&error);
        assert!(message.contains(missing_socket.to_str().unwrap()));
        assert!(message.contains("MAGI_CODEX_APP_SERVER_SOCKET"));
        assert!(message.contains("stdio://"));
    }

    #[tokio::test]
    async fn failed_batch_reports_last_successful_cursor() {
        let attempts = Arc::new(AtomicUsize::new(0));
        let seen_attempts = Arc::clone(&attempts);

        let result = deliver_messages_with_submitter(
            vec![message("1-0", "first"), message("2-0", "second")],
            "thread",
            Some("/tmp/project"),
            move |_thread_id, _cwd, _message_id, _text| {
                let seen_attempts = Arc::clone(&seen_attempts);
                async move {
                    if seen_attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                        Ok(())
                    } else {
                        Err(MagiError::CommandFailed("proxy down".to_string()))
                    }
                }
            },
        )
        .await
        .expect_err("second delivery should fail");

        assert_eq!(result.last_delivered_id.as_deref(), Some("1-0"));
        assert_eq!(attempts.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn first_failed_delivery_has_no_cursor_to_advance() {
        let result = deliver_messages_with_submitter(
            vec![message("1-0", "first")],
            "thread",
            Some("/tmp/project"),
            |_thread_id, _cwd, _message_id, _text| async {
                Err(MagiError::CommandFailed("proxy down".to_string()))
            },
        )
        .await
        .expect_err("first delivery should fail");

        assert!(result.last_delivered_id.is_none());
    }
}
