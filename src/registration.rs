//! Direct project/type registration management.

use std::path::PathBuf;

use crate::config::AppConfig;
use crate::error::{MagiError, Result};
use crate::team;

/// Add or refresh a project/type registration.
pub async fn add(
    team_name: String,
    agent: String,
    agent_type: String,
    project: PathBuf,
    session: Option<String>,
) -> Result<()> {
    let config = AppConfig::load()?;
    let url = configured_redis_url(&config)?;
    let project = project.display().to_string();
    team::register_agent_scoped_with_url(
        &url,
        &team_name,
        &agent,
        &agent_type,
        &project,
        session.as_deref(),
    )
    .await?;
    println!("Registered {agent} ({agent_type}) for {project} in team: {team_name}");
    Ok(())
}

/// Remove an agent and all registrations from a team.
pub async fn remove(team_name: String, agent: String) -> Result<()> {
    let config = AppConfig::load()?;
    let url = configured_redis_url(&config)?;
    team::remove_agent_with_url(&url, &team_name, &agent).await?;
    println!("Removed {agent} from team: {team_name}");
    Ok(())
}

/// Reset matching project/type registrations.
pub async fn reset(
    project: PathBuf,
    agent_type: String,
    agent: Option<String>,
    session: Option<String>,
) -> Result<()> {
    let config = AppConfig::load()?;
    let url = configured_redis_url(&config)?;
    let project = project.display().to_string();
    let removed = team::reset_registrations_with_url(
        &url,
        &project,
        &agent_type,
        agent.as_deref(),
        session.as_deref(),
    )
    .await?;
    println!("Removed {removed} registration(s)");
    Ok(())
}

fn configured_redis_url(config: &AppConfig) -> Result<String> {
    config
        .redis
        .url
        .clone()
        .ok_or_else(|| MagiError::InvalidConfig("redis.url is not configured".to_string()))
}
