//! Tests for Codex app-server bridge request construction and socket delivery.

use std::path::PathBuf;

use base64::prelude::*;
use magi::codex_bridge::{
    bridge_status_error_message, bridge_status_state_for_error, build_inject_items_request,
    build_turn_start_request, format_context_line, submit_to_codex_with_socket,
};
use serde_json::{json, Value};
use sha1::{Digest, Sha1};
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};

const WEBSOCKET_GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

#[test]
fn formats_context_line_for_codex_turns() {
    let message = magi::messaging::MessageRecord {
        id: "1-0".to_string(),
        event: magi::model::MessageEvent {
            from: "fatherly-balthasar".to_string(),
            to: "ecstatic-casper".to_string(),
            body: "hello".to_string(),
            created_at: "123".to_string(),
        },
    };

    assert_eq!(
        format_context_line(&message),
        "fatherly-balthasar->ecstatic-casper: hello"
    );
}

#[test]
fn builds_turn_start_request_with_cwd() {
    let request = build_turn_start_request(
        "thread-123",
        Some("/tmp/project"),
        "1700000000000-0",
        "alice->bob: status?",
    );

    assert_eq!(request["method"], "turn/start");
    assert_eq!(request["params"]["threadId"], "thread-123");
    assert_eq!(
        request["params"]["clientUserMessageId"],
        "magi-1700000000000-0"
    );
    assert_eq!(request["params"]["cwd"], "/tmp/project");
    assert_eq!(request["params"]["input"][0]["type"], "text");
    assert_eq!(request["params"]["input"][0]["text"], "alice->bob: status?");
}

#[test]
fn builds_user_message_thread_injection_request() {
    let request = build_inject_items_request("thread-123", "1-0", "alice->bob: status?");

    assert_eq!(request["method"], "thread/inject_items");
    assert_eq!(request["params"]["threadId"], "thread-123");
    assert_eq!(request["params"]["items"][0]["type"], "message");
    assert_eq!(request["params"]["items"][0]["role"], "user");
    assert_eq!(
        request["params"]["items"][0]["content"][0]["type"],
        "input_text"
    );
    assert_eq!(
        request["params"]["items"][0]["content"][0]["text"],
        "alice->bob: status?"
    );
}

#[test]
fn websocket_delivery_failures_are_reported_as_retrying_bridge_status() {
    let error = magi::error::MagiError::CommandFailed(
        "codex app-server websocket closed before JSON-RPC response".to_string(),
    );

    assert_eq!(bridge_status_state_for_error(&error), "retrying");
}

#[test]
fn missing_control_socket_is_reported_as_unsupported_bridge_status() {
    let error = magi::error::MagiError::UnsupportedRuntime(
        "Codex app-server control socket not found at /tmp/codex.sock; set MAGI_CODEX_APP_SERVER_SOCKET to a reachable Unix socket or start a managed Codex app-server daemon. stdio:// app-server processes cannot be reached by magi codex bridge."
            .to_string(),
    );

    assert_eq!(bridge_status_state_for_error(&error), "unsupported");
    let message = bridge_status_error_message(&error);
    assert!(message.contains("MAGI_CODEX_APP_SERVER_SOCKET"));
    assert!(message.contains("stdio://"));
}

#[tokio::test]
async fn submit_to_codex_starts_turn_after_injecting_thread_item() {
    let (_temp, socket, server) = spawn_fake_app_server(5, AppServerBehavior::AllOk).await;

    submit_to_codex_with_socket(
        "unused-codex",
        &socket,
        "thread-123",
        Some("/tmp/project"),
        "1-0",
        "alice->bob: status?",
    )
    .await
    .expect("delivery succeeds");

    let calls = server.await.expect("server task succeeds");
    let methods: Vec<_> = calls
        .iter()
        .filter_map(|call| call["method"].as_str())
        .collect();
    assert_eq!(
        methods,
        [
            "initialize",
            "initialized",
            "thread/resume",
            "thread/inject_items",
            "turn/start"
        ]
    );
    assert_eq!(calls[0]["params"]["capabilities"], Value::Null);
    assert!(calls[2]["params"].get("excludeTurns").is_none());
}

#[tokio::test]
async fn submit_to_codex_succeeds_when_turn_start_fails_after_injection() {
    let (_temp, socket, server) = spawn_fake_app_server(5, AppServerBehavior::TurnStartFails).await;

    submit_to_codex_with_socket(
        "unused-codex",
        &socket,
        "thread-123",
        Some("/tmp/project"),
        "1-0",
        "alice->bob: status?",
    )
    .await
    .expect("injection succeeds even when turn start fails");

    let calls = server.await.expect("server task succeeds");
    let methods: Vec<_> = calls
        .iter()
        .filter_map(|call| call["method"].as_str())
        .collect();
    assert_eq!(
        methods,
        [
            "initialize",
            "initialized",
            "thread/resume",
            "thread/inject_items",
            "turn/start"
        ]
    );
}

#[tokio::test]
async fn submit_to_codex_fails_without_turn_start_when_injection_fails() {
    let (_temp, socket, server) =
        spawn_fake_app_server(4, AppServerBehavior::InjectItemsFails).await;

    let error = submit_to_codex_with_socket(
        "unused-codex",
        &socket,
        "thread-123",
        Some("/tmp/project"),
        "1-0",
        "alice->bob: status?",
    )
    .await
    .expect_err("failed injection must fail delivery");

    assert!(
        error
            .to_string()
            .contains("codex app-server thread/inject_items failed"),
        "{error}"
    );
    let calls = server.await.expect("server task succeeds");
    let methods: Vec<_> = calls
        .iter()
        .filter_map(|call| call["method"].as_str())
        .collect();
    assert_eq!(
        methods,
        [
            "initialize",
            "initialized",
            "thread/resume",
            "thread/inject_items"
        ]
    );
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AppServerBehavior {
    AllOk,
    TurnStartFails,
    InjectItemsFails,
}

async fn spawn_fake_app_server(
    expected_messages: usize,
    behavior: AppServerBehavior,
) -> (TempDir, PathBuf, tokio::task::JoinHandle<Vec<Value>>) {
    let temp = tempfile::tempdir().expect("socket dir");
    let socket = temp.path().join("app-server.sock");
    let listener = UnixListener::bind(&socket).expect("bind fake app-server socket");
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept connection");
        accept_websocket_handshake(&mut stream).await;
        let mut calls = Vec::new();
        while calls.len() < expected_messages {
            let message = read_client_json_frame(&mut stream).await;
            let method = message["method"].as_str().unwrap_or_default().to_string();
            if let Some(id) = message.get("id").and_then(Value::as_u64) {
                let response = match (behavior, method.as_str()) {
                    (AppServerBehavior::TurnStartFails, "turn/start") => {
                        json!({ "id": id, "error": { "message": "active turn" } })
                    }
                    (AppServerBehavior::InjectItemsFails, "thread/inject_items") => {
                        json!({ "id": id, "error": { "message": "inject failed" } })
                    }
                    _ => json!({ "id": id, "result": {} }),
                };
                write_server_json_frame(&mut stream, &response).await;
            }
            calls.push(message);
        }
        calls
    });
    (temp, socket, server)
}

async fn accept_websocket_handshake(stream: &mut UnixStream) {
    let mut request = Vec::new();
    let mut byte = [0u8; 1];
    while !request.ends_with(b"\r\n\r\n") {
        stream.read_exact(&mut byte).await.expect("read handshake");
        request.push(byte[0]);
    }
    let request = String::from_utf8(request).expect("utf8 handshake");
    let key = request
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("sec-websocket-key")
                .then(|| value.trim())
        })
        .expect("sec-websocket-key");
    let accept = websocket_accept(key);
    let response = format!(
        "HTTP/1.1 101 Switching Protocols\r\n\
         Upgrade: websocket\r\n\
         Connection: Upgrade\r\n\
         Sec-WebSocket-Accept: {accept}\r\n\
         \r\n"
    );
    stream
        .write_all(response.as_bytes())
        .await
        .expect("write handshake");
}

fn websocket_accept(key: &str) -> String {
    let mut hasher = Sha1::new();
    hasher.update(key.as_bytes());
    hasher.update(WEBSOCKET_GUID.as_bytes());
    BASE64_STANDARD.encode(hasher.finalize())
}

async fn read_client_json_frame(stream: &mut UnixStream) -> Value {
    let mut header = [0u8; 2];
    stream
        .read_exact(&mut header)
        .await
        .expect("read frame header");
    assert_eq!(header[0] & 0x80, 0x80);
    assert_eq!(header[0] & 0x0F, 0x1);
    assert_eq!(header[1] & 0x80, 0x80);

    let mut length = u64::from(header[1] & 0x7F);
    if length == 126 {
        let mut extended = [0u8; 2];
        stream.read_exact(&mut extended).await.expect("read length");
        length = u64::from(u16::from_be_bytes(extended));
    } else if length == 127 {
        let mut extended = [0u8; 8];
        stream.read_exact(&mut extended).await.expect("read length");
        length = u64::from_be_bytes(extended);
    }

    let mut mask = [0u8; 4];
    stream.read_exact(&mut mask).await.expect("read mask");
    let mut payload = vec![0u8; length as usize];
    stream.read_exact(&mut payload).await.expect("read payload");
    for (index, byte) in payload.iter_mut().enumerate() {
        *byte ^= mask[index % 4];
    }
    serde_json::from_slice(&payload).expect("json frame")
}

async fn write_server_json_frame(stream: &mut UnixStream, value: &Value) {
    let payload = serde_json::to_vec(value).expect("serialize response");
    let mut frame = Vec::new();
    frame.push(0x81);
    if payload.len() <= 125 {
        frame.push(payload.len() as u8);
    } else if payload.len() <= u16::MAX as usize {
        frame.push(126);
        frame.extend_from_slice(&(payload.len() as u16).to_be_bytes());
    } else {
        frame.push(127);
        frame.extend_from_slice(&(payload.len() as u64).to_be_bytes());
    }
    frame.extend_from_slice(&payload);
    stream.write_all(&frame).await.expect("write response");
}
