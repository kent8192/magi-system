//! Project/type identity discovery commands.

use std::path::PathBuf;

use crate::config::AppConfig;
use crate::error::{MagiError, Result};
use crate::team;

/// List all identities registered for a project/type pair.
pub async fn list(project: PathBuf, agent_type: String) -> Result<()> {
    let config = AppConfig::load()?;
    let url = configured_redis_url(&config)?;
    let project = project.display().to_string();
    let registrations = team::list_registrations_with_url(&url, &project, &agent_type).await?;
    for registration in registrations {
        println!("{}", registration.agent);
    }
    Ok(())
}

/// Resolve the current project/type identity state.
pub async fn whoami(project: PathBuf, agent_type: String) -> Result<()> {
    let config = AppConfig::load()?;
    let url = configured_redis_url(&config)?;
    let project = project.display().to_string();
    let registrations = team::list_registrations_with_url(&url, &project, &agent_type).await?;
    match registrations.len() {
        1 => println!("{}", registrations[0].agent),
        n if n > 1 => {
            println!("multiple matches");
            for registration in registrations {
                println!("{}", registration.agent);
            }
        }
        _ => {
            let suggestions =
                team::suggested_registrations_with_url(&url, &project, &agent_type).await?;
            if suggestions.is_empty() {
                println!("not joined");
            } else {
                println!("no exact match; suggested matches");
                for registration in suggestions {
                    println!("{} {}", registration.agent, registration.project);
                }
            }
        }
    }
    Ok(())
}

fn configured_redis_url(config: &AppConfig) -> Result<String> {
    config
        .redis
        .url
        .clone()
        .ok_or_else(|| MagiError::InvalidConfig("redis.url is not configured".to_string()))
}
