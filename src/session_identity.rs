//! Runtime session identity resolution for interactive agent sessions.
//!
//! Session hooks create a small record keyed by the runtime session id. When a
//! `magi` command is executed inside that session, the record supplies the
//! agent name. Persistent config stores only the active team, so concurrent
//! sessions cannot overwrite each other's agent identity.

use std::env;
use std::fs;
use std::path::PathBuf;

use crate::config::AppConfig;

/// Active team/agent identity resolved for the current command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveIdentity {
    /// Team name used to scope Redis keys.
    pub team: Option<String>,
    /// Agent name used as the sender and inbox cursor owner.
    pub agent: Option<String>,
}

/// Resolve the active identity for the current process environment.
pub fn resolve_identity(config: &AppConfig) -> ActiveIdentity {
    resolve_identity_with_env(config, |key| env::var(key).ok())
}

/// Resolve identity using an injected environment reader.
///
/// This is exposed for tests so they can assert session-record precedence
/// without mutating the process-wide environment.
pub fn resolve_identity_with_env<F>(config: &AppConfig, mut env: F) -> ActiveIdentity
where
    F: FnMut(&str) -> Option<String>,
{
    let mut identity = ActiveIdentity {
        team: config.identity.active_team.clone(),
        agent: None,
    };

    if let Some(session) = session_identity_from_env(&mut env) {
        if session.team.is_some() {
            identity.team = session.team;
        }
        if session.agent.is_some() {
            identity.agent = session.agent;
        }
    }

    identity
}

fn session_identity_from_env<F>(env: &mut F) -> Option<ActiveIdentity>
where
    F: FnMut(&str) -> Option<String>,
{
    let session_id = first_non_empty_env(
        env,
        &[
            "MAGI_SESSION_ID",
            "CODEX_THREAD_ID",
            "CODEX_SESSION_ID",
            "CLAUDE_SESSION_ID",
        ],
    )?;
    let session_key = sanitize_session_key(&session_id)?;

    for state_dir in state_dirs(env) {
        let session_file = state_dir
            .join("sessions")
            .join(format!("{session_key}.agent"));
        if let Some(identity) = read_session_file(session_file) {
            return Some(identity);
        }
    }

    None
}

fn first_non_empty_env<F>(env: &mut F, keys: &[&str]) -> Option<String>
where
    F: FnMut(&str) -> Option<String>,
{
    keys.iter()
        .filter_map(|key| env(key))
        .map(|value| value.trim().to_string())
        .find(|value| !value.is_empty())
}

fn state_dirs<F>(env: &mut F) -> Vec<PathBuf>
where
    F: FnMut(&str) -> Option<String>,
{
    let mut dirs = Vec::new();

    if let Some(dir) = non_empty_env(env, "MAGI_CODEX_STATE_DIR") {
        dirs.push(PathBuf::from(dir));
    }
    if let Some(dir) = non_empty_env(env, "MAGI_AGENT_STATE_DIR") {
        dirs.push(PathBuf::from(dir));
    }

    if let Some(xdg_state_home) = non_empty_env(env, "XDG_STATE_HOME") {
        let root = PathBuf::from(xdg_state_home);
        dirs.push(root.join("magi-codex"));
        dirs.push(root.join("magi-agent"));
    } else if let Some(home) = non_empty_env(env, "HOME") {
        let root = PathBuf::from(home).join(".local").join("state");
        dirs.push(root.join("magi-codex"));
        dirs.push(root.join("magi-agent"));
    }

    dedup_paths(dirs)
}

fn non_empty_env<F>(env: &mut F, key: &str) -> Option<String>
where
    F: FnMut(&str) -> Option<String>,
{
    env(key)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn dedup_paths(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut unique = Vec::new();
    for path in paths {
        if !unique.iter().any(|existing| existing == &path) {
            unique.push(path);
        }
    }
    unique
}

fn sanitize_session_key(session_id: &str) -> Option<String> {
    let key: String = session_id
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-'))
        .collect();
    (!key.is_empty()).then_some(key)
}

fn read_session_file(path: PathBuf) -> Option<ActiveIdentity> {
    let content = fs::read_to_string(path).ok()?;
    let mut lines = content.lines();
    let agent = non_empty_line(lines.next());
    let team = non_empty_line(lines.next());

    if agent.is_none() && team.is_none() {
        return None;
    }

    Some(ActiveIdentity { team, agent })
}

fn non_empty_line(line: Option<&str>) -> Option<String> {
    line.map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}
