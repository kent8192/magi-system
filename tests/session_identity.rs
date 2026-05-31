//! Tests for resolving the active identity from per-session hook state.
//!
//! These tests use explicit environment maps and temporary state directories so
//! they never mutate the process environment or the real magi state.

use std::collections::HashMap;
use std::fs;

use magi::config::AppConfig;
use magi::session_identity::resolve_identity_with_env;

#[test]
fn codex_session_record_supplies_session_identity() {
    let temp = tempfile::tempdir().expect("tempdir");
    let sessions = temp.path().join("sessions");
    fs::create_dir_all(&sessions).expect("sessions dir");
    fs::write(
        sessions.join("thread-1.agent"),
        "session-agent\nsession-team\n",
    )
    .expect("session file");

    let mut config = AppConfig::default();
    config.identity.active_team = Some("global-team".to_string());

    let mut env = HashMap::new();
    env.insert("CODEX_THREAD_ID", "thread-1");
    env.insert(
        "MAGI_CODEX_STATE_DIR",
        temp.path().to_str().expect("utf-8 path"),
    );

    let identity = resolve_identity_with_env(&config, |key| {
        env.get(key).map(|value| (*value).to_string())
    });

    assert_eq!(identity.agent.as_deref(), Some("session-agent"));
    assert_eq!(identity.team.as_deref(), Some("session-team"));
}

#[test]
fn missing_session_record_has_no_agent_fallback() {
    let temp = tempfile::tempdir().expect("tempdir");

    let mut config = AppConfig::default();
    config.identity.active_team = Some("global-team".to_string());

    let mut env = HashMap::new();
    env.insert("CODEX_THREAD_ID", "thread-missing");
    env.insert(
        "MAGI_CODEX_STATE_DIR",
        temp.path().to_str().expect("utf-8 path"),
    );

    let identity = resolve_identity_with_env(&config, |key| {
        env.get(key).map(|value| (*value).to_string())
    });

    assert_eq!(identity.agent, None);
    assert_eq!(identity.team.as_deref(), Some("global-team"));
}
