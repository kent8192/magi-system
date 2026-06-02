//! Actas-style exclusive role claims.

use redis::AsyncCommands;

use crate::config::AppConfig;
use crate::error::{MagiError, Result};
use crate::model::RedisKeys;
use crate::redis_client;
use crate::session_identity::resolve_identity;

/// Claim an agent role for a session.
pub async fn claim(
    agent: String,
    team: Option<String>,
    session: Option<String>,
    ttl: u64,
) -> Result<()> {
    let config = AppConfig::load()?;
    let url = configured_redis_url(&config)?;
    let identity = resolve_identity(&config);
    let team = team
        .or(identity.team)
        .ok_or_else(|| MagiError::InvalidConfig("identity.active_team is required".to_string()))?;
    let session = session
        .or_else(session_from_env)
        .ok_or_else(|| MagiError::InvalidConfig("session id is required".to_string()))?;
    claim_with_url(&url, &team, &agent, &session, ttl).await?;
    println!("Claimed {agent} for session {session}");
    Ok(())
}

/// Release an agent role claim.
pub async fn release(agent: String, team: Option<String>, session: Option<String>) -> Result<()> {
    let config = AppConfig::load()?;
    let url = configured_redis_url(&config)?;
    let identity = resolve_identity(&config);
    let team = team
        .or(identity.team)
        .ok_or_else(|| MagiError::InvalidConfig("identity.active_team is required".to_string()))?;
    let session = session
        .or_else(session_from_env)
        .ok_or_else(|| MagiError::InvalidConfig("session id is required".to_string()))?;
    release_with_url(&url, &team, &agent, &session).await?;
    println!("Released {agent} for session {session}");
    Ok(())
}

/// Print role claim status.
pub async fn status(agent: String, team: Option<String>) -> Result<()> {
    let config = AppConfig::load()?;
    let url = configured_redis_url(&config)?;
    let identity = resolve_identity(&config);
    let team = team
        .or(identity.team)
        .ok_or_else(|| MagiError::InvalidConfig("identity.active_team is required".to_string()))?;
    match status_with_url(&url, &team, &agent).await? {
        Some(session) => println!("claimed {agent} {session}"),
        None => println!("unclaimed {agent}"),
    }
    Ok(())
}

/// Actas locks use Redis TTL, so GC is a no-op health command.
pub async fn gc() -> Result<()> {
    println!("Actas locks use Redis TTL; gc complete");
    Ok(())
}

/// Enforce that the current session owns a role if it is claimed.
pub async fn ensure_unblocked_for_session(url: &str, team: &str, agent: &str) -> Result<()> {
    if let Some(owner) = status_with_url(url, team, agent).await? {
        if session_from_env().as_deref() != Some(owner.as_str()) {
            return Err(MagiError::InvalidConfig(format!(
                "agent `{agent}` is claimed by another live session"
            )));
        }
    }
    Ok(())
}

pub async fn claim_with_url(
    url: &str,
    team: &str,
    agent: &str,
    session: &str,
    ttl: u64,
) -> Result<()> {
    let keys = RedisKeys::new(team);
    let key = keys.actas_lock(agent);
    let mut connection = redis_client::connect(url).await?;
    let existing: Option<String> = connection.get(&key).await?;
    if let Some(existing) = existing {
        if existing != session {
            return Err(MagiError::InvalidConfig(format!(
                "agent `{agent}` is already claimed by session `{existing}`"
            )));
        }
    }
    let _: () = redis::cmd("SET")
        .arg(&key)
        .arg(session)
        .arg("EX")
        .arg(ttl.max(1))
        .query_async(&mut connection)
        .await?;
    Ok(())
}

pub async fn release_with_url(url: &str, team: &str, agent: &str, session: &str) -> Result<()> {
    let keys = RedisKeys::new(team);
    let key = keys.actas_lock(agent);
    let mut connection = redis_client::connect(url).await?;
    let existing: Option<String> = connection.get(&key).await?;
    if existing.as_deref() != Some(session) {
        return Err(MagiError::InvalidConfig(format!(
            "agent `{agent}` is not claimed by session `{session}`"
        )));
    }
    let _: () = connection.del(key).await?;
    Ok(())
}

pub async fn status_with_url(url: &str, team: &str, agent: &str) -> Result<Option<String>> {
    let keys = RedisKeys::new(team);
    let mut connection = redis_client::connect(url).await?;
    Ok(connection.get(keys.actas_lock(agent)).await?)
}

fn configured_redis_url(config: &AppConfig) -> Result<String> {
    config
        .redis
        .url
        .clone()
        .ok_or_else(|| MagiError::InvalidConfig("redis.url is not configured".to_string()))
}

fn session_from_env() -> Option<String> {
    std::env::var("MAGI_SESSION_ID")
        .or_else(|_| std::env::var("CODEX_THREAD_ID"))
        .or_else(|_| std::env::var("CODEX_SESSION_ID"))
        .ok()
        .filter(|value| !value.is_empty())
}
