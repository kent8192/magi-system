//! Tests for Codex app-server bridge request construction and proxy fallback.

use std::io::Write;

use magi::codex_bridge::{
    build_inject_items_request, build_turn_start_request, format_context_line, submit_to_codex,
};
use tempfile::NamedTempFile;

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
fn builds_user_message_fallback_injection_request() {
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

#[tokio::test]
async fn submit_to_codex_falls_back_to_inject_items_when_turn_start_fails() {
    let mut script = NamedTempFile::new().expect("temp script");
    writeln!(
        script,
        r#"#!/usr/bin/env bash
set -euo pipefail
while IFS= read -r line; do
  printf '%s\n' "$line" >>"$CODEX_PROXY_LOG"
  id="$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')"
  case "$line" in
    *'"method":"turn/start"'*)
      printf '{{"id":%s,"error":{{"message":"active turn"}}}}\n' "$id"
      ;;
    *)
      printf '{{"id":%s,"result":{{}}}}\n' "$id"
      ;;
  esac
done
"#
    )
    .expect("write script");
    let script = script.into_temp_path();
    std::fs::set_permissions(&script, {
        let mut permissions = std::fs::metadata(&script).unwrap().permissions();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            permissions.set_mode(0o755);
        }
        permissions
    })
    .expect("chmod script");

    let log = NamedTempFile::new().expect("temp log");
    std::env::set_var("CODEX_PROXY_LOG", log.path());

    submit_to_codex(
        script.to_str().unwrap(),
        "thread-123",
        Some("/tmp/project"),
        "1-0",
        "alice->bob: status?",
    )
    .await
    .expect("fallback injection succeeds");

    let calls = std::fs::read_to_string(log.path()).expect("read proxy log");
    assert!(calls.contains(r#""method":"initialize""#));
    assert!(calls.contains(r#""method":"thread/resume""#));
    assert!(calls.contains(r#""method":"turn/start""#));
    assert!(calls.contains(r#""method":"thread/inject_items""#));
}
