//! Tests for resolving the active identity from per-session hook state.
//!
//! These tests use explicit environment maps and temporary state directories so
//! they never mutate the process environment or the real magi state.

use std::collections::HashMap;
use std::fs;

use magi::config::AppConfig;
use magi::session_identity::{missing_session_agent_message_with_env, resolve_identity_with_env};

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

#[test]
fn codex_current_pointer_supplies_identity_without_session_env() {
    let temp = tempfile::tempdir().expect("tempdir");
    let current = temp.path().join("current");
    fs::create_dir_all(&current).expect("current dir");
    fs::write(current.join("project-1.agent"), "hook-agent\nhook-team\n").expect("current pointer");

    let mut config = AppConfig::default();
    config.identity.active_team = Some("global-team".to_string());

    let mut env = HashMap::new();
    env.insert(
        "MAGI_CODEX_STATE_DIR",
        temp.path().to_str().expect("utf-8 path"),
    );
    env.insert("PWD", "project-1");

    let identity = resolve_identity_with_env(&config, |key| {
        env.get(key).map(|value| (*value).to_string())
    });

    assert_eq!(identity.agent.as_deref(), Some("hook-agent"));
    assert_eq!(identity.team.as_deref(), Some("hook-team"));
}

#[test]
fn explicit_session_record_wins_over_codex_current_pointer() {
    let temp = tempfile::tempdir().expect("tempdir");
    let sessions = temp.path().join("sessions");
    let current = temp.path().join("current");
    fs::create_dir_all(&sessions).expect("sessions dir");
    fs::create_dir_all(&current).expect("current dir");
    fs::write(
        sessions.join("thread-1.agent"),
        "session-agent\nsession-team\n",
    )
    .expect("session file");
    fs::write(current.join("project-1.agent"), "hook-agent\nhook-team\n").expect("current pointer");

    let mut config = AppConfig::default();
    config.identity.active_team = Some("global-team".to_string());

    let mut env = HashMap::new();
    env.insert("CODEX_THREAD_ID", "thread-1");
    env.insert(
        "MAGI_CODEX_STATE_DIR",
        temp.path().to_str().expect("utf-8 path"),
    );
    env.insert("PWD", "project-1");

    let identity = resolve_identity_with_env(&config, |key| {
        env.get(key).map(|value| (*value).to_string())
    });

    assert_eq!(identity.agent.as_deref(), Some("session-agent"));
    assert_eq!(identity.team.as_deref(), Some("session-team"));
}

#[test]
fn missing_session_agent_message_warns_when_codex_hooks_are_disabled() {
    let temp = tempfile::tempdir().expect("tempdir");
    let codex_dir = temp.path().join(".codex");
    fs::create_dir_all(&codex_dir).expect("codex dir");
    fs::write(
        codex_dir.join("config.toml"),
        "[features]\nhooks = false\nplugin_hooks = false\n",
    )
    .expect("codex config");

    let mut env = HashMap::new();
    env.insert("CODEX_THREAD_ID", "thread-missing");
    env.insert("HOME", temp.path().to_str().expect("utf-8 path"));

    let message = missing_session_agent_message_with_env(|key| {
        env.get(key).map(|value| (*value).to_string())
    });

    assert!(message.contains("session agent is required"));
    assert!(message.contains("Codex hooks are disabled"));
    assert!(message.contains("features.hooks=false"));
    assert!(message.contains("features.plugin_hooks=false"));
}
