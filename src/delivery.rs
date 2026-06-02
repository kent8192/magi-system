//! Delivery mode configuration commands.

use std::path::PathBuf;

use redis::AsyncCommands;

use crate::cli::DeliveryMode;
use crate::config::AppConfig;
use crate::error::{MagiError, Result};
use crate::model::RedisKeys;
use crate::redis_client;
use crate::team;

/// Set delivery mode for a project/type pair.
pub async fn set(mode: DeliveryMode, agent_type: String, project: PathBuf) -> Result<()> {
    set_mode(mode, agent_type, project).await
}

/// Show delivery mode for a project/type pair.
pub async fn status(agent_type: Option<String>, project: Option<PathBuf>) -> Result<()> {
    let config = AppConfig::load()?;
    let url = configured_redis_url(&config)?;
    let mut connection = redis_client::connect(&url).await?;
    if let (Some(agent_type), Some(project)) = (agent_type, project) {
        let project = project.display().to_string();
        let key = RedisKeys::delivery(&agent_type, &project);
        let fields: std::collections::HashMap<String, String> =
            connection.hgetall(key).await.unwrap_or_default();
        if fields.is_empty() {
            println!("off {agent_type} {project}");
        } else {
            println!(
                "{} {} {}",
                fields.get("mode").map(String::as_str).unwrap_or("off"),
                agent_type,
                project
            );
        }
    } else {
        return Err(MagiError::InvalidConfig(
            "delivery status requires --type and --project".to_string(),
        ));
    }
    Ok(())
}

/// Refresh delivery configuration.
pub async fn restart(agent_type: String, project: PathBuf) -> Result<()> {
    set_mode(DeliveryMode::Both, agent_type, project).await
}

/// Disable delivery for a project/type pair.
pub async fn stop(agent_type: String, project: PathBuf) -> Result<()> {
    set_mode(DeliveryMode::Off, agent_type, project).await
}

async fn set_mode(mode: DeliveryMode, agent_type: String, project: PathBuf) -> Result<()> {
    let config = AppConfig::load()?;
    let url = configured_redis_url(&config)?;
    let project = project.display().to_string();
    let mode = mode_string(mode);
    let key = RedisKeys::delivery(&agent_type, &project);
    let mut connection = redis_client::connect(&url).await?;
    let _: () = redis::pipe()
        .atomic()
        .hset(&key, "mode", mode)
        .hset(&key, "type", &agent_type)
        .hset(&key, "project", &project)
        .hset(&key, "updated_at", team::unix_timestamp_string())
        .query_async(&mut connection)
        .await?;
    println!("{mode} {agent_type} {project}");
    Ok(())
}

fn mode_string(mode: DeliveryMode) -> &'static str {
    match mode {
        DeliveryMode::Monitor => "monitor",
        DeliveryMode::Turn => "turn",
        DeliveryMode::Both => "both",
        DeliveryMode::Off => "off",
    }
}

fn configured_redis_url(config: &AppConfig) -> Result<String> {
    config
        .redis
        .url
        .clone()
        .ok_or_else(|| MagiError::InvalidConfig("redis.url is not configured".to_string()))
}
