//! Team membership operations for the `magi` CLI.
//!
//! A *team* groups the CLI AI agents that can message one another. This module
//! owns the lifecycle of teams and their members on top of Redis: creating a
//! team (and registering its initial owner), listing all known teams, listing
//! the members of a team, and registering or refreshing an individual agent's
//! membership.
//!
//! All persistent state lives in Redis under keys derived from `RedisKeys`.
//! The functions here come in two flavours:
//!
//! - High-level entry points (`create`, `list`, `members`) that load the
//!   `AppConfig` from `~/.magi`, resolve the configured Redis URL, and print
//!   human-readable output. These back the corresponding `magi team` CLI
//!   subcommands.
//! - Lower-level `*_with_url` / `*_with_connection` helpers that take an
//!   explicit Redis URL or connection. These contain the actual Redis logic and
//!   are reused by other modules (for example, invite-based onboarding) and by
//!   tests that need to target a specific Redis instance.
//!
//! Membership data is stored across several Redis structures:
//! a global set of team names, a per-team hash of metadata, a per-team set of
//! agent names, a per-agent hash of profile fields, and a per-agent set of
//! `type:project` registration tuples.

use redis::AsyncCommands;
use std::collections::HashMap;

use crate::config::AppConfig;
use crate::error::{MagiError, Result};
use crate::model::RedisKeys;
use crate::redis_client;
use crate::session_identity::resolve_identity;

/// A single agent's membership within a team, as reconstructed from Redis.
///
/// Each field mirrors data persisted under the per-agent Redis hash (see
/// `RedisKeys::agent`) plus the most recent registration tuple. Equality is
/// derived so members can be compared directly in tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TeamMember {
    /// The agent's unique name within the team (the Redis set member value).
    pub name: String,
    /// The agent's kind/category (for example, the CLI agent type), stored in
    /// the agent hash under the `type` field.
    pub agent_type: String,
    /// The project the agent is most recently registered against, parsed from
    /// the latest `type:project` registration tuple (empty when none exist).
    pub project: String,
    /// Unix-epoch-seconds timestamp (as a string) of when the agent was first
    /// registered.
    pub created_at: String,
    /// Unix-epoch-seconds timestamp (as a string) of the agent's most recent
    /// registration/heartbeat.
    pub last_seen_at: String,
}

/// A scoped project/type registration for one agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentRegistration {
    /// Agent name that owns the registration.
    pub agent: String,
    /// Agent runtime type recorded for the registration.
    pub agent_type: String,
    /// Project path recorded for the registration.
    pub project: String,
    /// Optional runtime session id associated with this registration.
    pub session: Option<String>,
}

/// Creates a new team and registers the current agent as its owner.
///
/// This is the entry point for the `magi team create` subcommand. It loads the
/// local `AppConfig`, resolves the configured Redis URL, and derives the
/// owner name from the current session identity (falling back to `"owner"` when
/// no session agent is set). On success it prints a confirmation line.
///
/// # Errors
///
/// Returns an error if the config cannot be loaded, if `redis.url` is not
/// configured, if the Redis connection fails, or if a team with the same name
/// already exists (see `create_team_with_url`).
pub async fn create(name: String) -> Result<()> {
    let config = AppConfig::load()?;
    let url = configured_redis_url(&config)?;
    // Use the session agent identity as the team owner; default to "owner"
    // when the command is run outside a spawned session.
    let owner = resolve_identity(&config)
        .agent
        .unwrap_or_else(|| "owner".to_string());

    create_team_with_url(&url, &name, &owner).await?;
    println!("Created team: {name}");
    Ok(())
}

/// Lists every known team, one per line.
///
/// Backs the `magi team list` subcommand. Team names are read from the global
/// Redis set returned by `RedisKeys::teams`; an empty team prefix is used here
/// because that set is not scoped to any particular team.
///
/// # Errors
///
/// Returns an error if the config cannot be loaded, if `redis.url` is not
/// configured, or if the Redis connection/read fails.
pub async fn list() -> Result<()> {
    let config = AppConfig::load()?;
    let url = configured_redis_url(&config)?;
    let mut connection = redis_client::connect(&url).await?;
    // The global team-name set is not team-scoped, so an empty prefix is fine.
    let keys = RedisKeys::new("");
    let teams: Vec<String> = connection.smembers(keys.teams()).await?;

    for team in teams {
        println!("{team}");
    }

    Ok(())
}

/// Prints the members of a team along with a total count.
///
/// Backs the `magi team members` subcommand. When `name` is `None`, the team is
/// taken from the active team configured in the local identity. Each member is
/// printed as `name (type) - project`.
///
/// # Errors
///
/// Returns an error if the config cannot be loaded, if `redis.url` is not
/// configured, if neither an explicit `name` nor an active team is available
/// (`MagiError::InvalidConfig`), or if the Redis read fails.
pub async fn members(name: Option<String>) -> Result<()> {
    let config = AppConfig::load()?;
    let url = configured_redis_url(&config)?;
    // Resolve the target team: explicit argument wins, otherwise fall back to
    // the active team from the configured identity.
    let team = name
        .or(config.identity.active_team)
        .ok_or_else(|| MagiError::InvalidConfig("team is required".to_string()))?;
    let members = list_members_with_url(&url, &team).await?;

    println!("Team: {team}");
    println!();
    for member in &members {
        println!(
            "  {} ({}) - {}",
            member.name, member.agent_type, member.project
        );
    }
    println!();
    println!("{} member(s)", members.len());

    Ok(())
}

/// CLI entry point for `magi team rename`.
pub async fn rename(old: String, new: String) -> Result<()> {
    let config = AppConfig::load()?;
    let url = configured_redis_url(&config)?;
    rename_team_with_url(&url, &old, &new).await?;
    println!("Renamed team {old} -> {new}");
    Ok(())
}

/// Creates a team on a specific Redis instance and registers its `owner`.
///
/// This is the connection-explicit core used by `create` and by callers (such
/// as onboarding flows and tests) that already know the Redis URL. It claims the
/// team name, writes the team metadata hash, and registers the owner as the
/// first member, all guarded against partial failure.
///
/// # Errors
///
/// Returns `MagiError::InvalidConfig` if a team named `team` already exists, and
/// propagates any Redis connection or command error. On a failed metadata write
/// the team-name claim is rolled back before the error is returned, so a later
/// attempt can recreate the team cleanly.
pub async fn create_team_with_url(url: &str, team: &str, owner: &str) -> Result<()> {
    let keys = RedisKeys::new(team);
    let mut connection = redis_client::connect(url).await?;

    // Claim the team name atomically. SADD reports how many members were newly
    // added, so a return of 0 means the team already exists and we must not
    // overwrite its owner or timestamps.
    let added: i64 = connection.sadd(keys.teams(), team).await?;
    if added == 0 {
        return Err(MagiError::InvalidConfig(format!(
            "team `{team}` already exists"
        )));
    }

    let now = unix_timestamp_string();
    // Build a single MULTI/EXEC transaction so the team-metadata hash and the
    // owner's registration either all apply or none do.
    let mut pipe = redis::pipe();
    pipe.atomic()
        .hset(keys.team(), "name", team)
        .hset(keys.team(), "owner", owner)
        .hset(keys.team(), "created_at", &now)
        .hset(keys.team(), "updated_at", &now);
    // Register the owner with an "owner" type and no project; reuse the same
    // timestamp for both created_at and last_seen_at on first registration.
    add_agent_registration_to_pipe(&mut pipe, &keys, owner, "owner", "", &now, &now);

    let result: redis::RedisResult<()> = pipe.query_async(&mut connection).await;
    if let Err(error) = result {
        // Roll back the team-name claim so the partially created team can be
        // recreated by a later attempt.
        let _: redis::RedisResult<i64> = connection.srem(keys.teams(), team).await;
        return Err(error.into());
    }

    Ok(())
}

/// Registers (or refreshes) an agent's membership in a team via a URL.
///
/// Opens a Redis connection to `url` and delegates to
/// `register_agent_with_connection`. Used by onboarding/invite flows that
/// supply the team, agent name, agent type, and current project explicitly.
///
/// # Errors
///
/// Returns an error if the Redis connection fails or the underlying
/// registration pipeline fails.
pub async fn register_agent_with_url(
    url: &str,
    team: &str,
    agent: &str,
    agent_type: &str,
    project: &str,
) -> Result<()> {
    let keys = RedisKeys::new(team);
    let mut connection = redis_client::connect(url).await?;
    register_agent_with_connection(&mut connection, &keys, agent, agent_type, project).await
}

/// Registers an agent and optionally associates the project/type tuple with a session id.
pub async fn register_agent_scoped_with_url(
    url: &str,
    team: &str,
    agent: &str,
    agent_type: &str,
    project: &str,
    session: Option<&str>,
) -> Result<()> {
    let keys = RedisKeys::new(team);
    let mut connection = redis_client::connect(url).await?;
    register_agent_with_connection(&mut connection, &keys, agent, agent_type, project).await?;
    if let Some(session) = session.filter(|session| !session.is_empty()) {
        let _: () = connection
            .hset(
                keys.registration_sessions(agent),
                registration_tuple(agent_type, project),
                session,
            )
            .await?;
    }
    Ok(())
}

/// Removes an entire agent from a team, including profile, registrations, and cursor state.
pub async fn remove_agent_with_url(url: &str, team: &str, agent: &str) -> Result<()> {
    let keys = RedisKeys::new(team);
    let mut connection = redis_client::connect(url).await?;
    remove_agent_with_connection(&mut connection, &keys, team, agent).await
}

/// Removes registrations matching the supplied filters.
pub async fn reset_registrations_with_url(
    url: &str,
    project: &str,
    agent_type: &str,
    agent: Option<&str>,
    session: Option<&str>,
) -> Result<usize> {
    let mut connection = redis_client::connect(url).await?;
    let global_keys = RedisKeys::new("");
    let teams: Vec<String> = connection.smembers(global_keys.teams()).await?;
    let mut removed = 0;

    for team in teams {
        let keys = RedisKeys::new(&team);
        let agents = if let Some(agent) = agent {
            vec![agent.to_string()]
        } else {
            let mut agents: Vec<String> = connection.smembers(keys.team_agents()).await?;
            agents.sort();
            agents
        };

        for agent in agents {
            if remove_registration_with_connection(
                &mut connection,
                &keys,
                &team,
                &agent,
                agent_type,
                project,
                session,
            )
            .await?
            {
                removed += 1;
            }
        }
    }

    Ok(removed)
}

/// Lists registrations matching a project/type pair across all known teams.
pub async fn list_registrations_with_url(
    url: &str,
    project: &str,
    agent_type: &str,
) -> Result<Vec<AgentRegistration>> {
    let mut connection = redis_client::connect(url).await?;
    let global_keys = RedisKeys::new("");
    let teams: Vec<String> = connection.smembers(global_keys.teams()).await?;
    let mut matches = Vec::new();

    for team in teams {
        let keys = RedisKeys::new(&team);
        let agents: Vec<String> = connection.smembers(keys.team_agents()).await?;
        for agent in agents {
            if let Some(registration) =
                registration_for_agent(&mut connection, &keys, &agent, agent_type, project).await?
            {
                matches.push(registration);
            }
        }
    }

    matches.sort_by(|a, b| a.agent.cmp(&b.agent));
    Ok(matches)
}

/// Lists same-type registrations for projects other than the requested one.
pub async fn suggested_registrations_with_url(
    url: &str,
    project: &str,
    agent_type: &str,
) -> Result<Vec<AgentRegistration>> {
    let mut connection = redis_client::connect(url).await?;
    let global_keys = RedisKeys::new("");
    let teams: Vec<String> = connection.smembers(global_keys.teams()).await?;
    let mut matches = Vec::new();

    for team in teams {
        let keys = RedisKeys::new(&team);
        let agents: Vec<String> = connection.smembers(keys.team_agents()).await?;
        for agent in agents {
            for registration in registrations_for_agent(&mut connection, &keys, &agent).await? {
                if registration.agent_type == agent_type && registration.project != project {
                    matches.push(registration);
                }
            }
        }
    }

    matches.sort_by(|a, b| a.agent.cmp(&b.agent).then(a.project.cmp(&b.project)));
    Ok(matches)
}

/// Renames one agent while preserving profile, registrations, and inbox cursor.
pub async fn rename_agent_with_url(url: &str, team: &str, old: &str, new: &str) -> Result<()> {
    let keys = RedisKeys::new(team);
    let mut connection = redis_client::connect(url).await?;
    let exists: bool = connection.sismember(keys.team_agents(), old).await?;
    if !exists {
        return Err(MagiError::NotFound(format!("agent `{old}`")));
    }
    let collision: bool = connection.sismember(keys.team_agents(), new).await?;
    if collision {
        return Err(MagiError::InvalidConfig(format!(
            "agent `{new}` already exists"
        )));
    }

    rename_existing_key(&mut connection, keys.agent(old), keys.agent(new)).await?;
    rename_existing_key(
        &mut connection,
        keys.registrations(old),
        keys.registrations(new),
    )
    .await?;
    rename_existing_key(
        &mut connection,
        keys.registration_sessions(old),
        keys.registration_sessions(new),
    )
    .await?;
    rename_existing_key(&mut connection, keys.cursor(old), keys.cursor(new)).await?;

    let _: () = redis::pipe()
        .atomic()
        .srem(keys.team_agents(), old)
        .sadd(keys.team_agents(), new)
        .hset(keys.agent(new), "name", new)
        .hset(keys.team(), "updated_at", unix_timestamp_string())
        .query_async(&mut connection)
        .await?;

    Ok(())
}

/// Renames a team and moves all team-scoped keys to the new name.
pub async fn rename_team_with_url(url: &str, old: &str, new: &str) -> Result<()> {
    let old_keys = RedisKeys::new(old);
    let new_keys = RedisKeys::new(new);
    let mut connection = redis_client::connect(url).await?;
    let exists: bool = connection.sismember(old_keys.teams(), old).await?;
    if !exists {
        return Err(MagiError::NotFound(format!("team `{old}`")));
    }
    let collision: bool = connection.sismember(old_keys.teams(), new).await?;
    if collision {
        return Err(MagiError::InvalidConfig(format!(
            "team `{new}` already exists"
        )));
    }

    let agents: Vec<String> = connection.smembers(old_keys.team_agents()).await?;
    for agent in &agents {
        rename_existing_key(
            &mut connection,
            old_keys.agent(agent),
            new_keys.agent(agent),
        )
        .await?;
        rename_existing_key(
            &mut connection,
            old_keys.registrations(agent),
            new_keys.registrations(agent),
        )
        .await?;
        rename_existing_key(
            &mut connection,
            old_keys.registration_sessions(agent),
            new_keys.registration_sessions(agent),
        )
        .await?;
        rename_existing_key(
            &mut connection,
            old_keys.cursor(agent),
            new_keys.cursor(agent),
        )
        .await?;
    }
    rename_existing_key(
        &mut connection,
        old_keys.team_agents(),
        new_keys.team_agents(),
    )
    .await?;
    rename_existing_key(&mut connection, old_keys.team(), new_keys.team()).await?;
    rename_existing_key(&mut connection, old_keys.stream(), new_keys.stream()).await?;

    let _: () = redis::pipe()
        .atomic()
        .srem(old_keys.teams(), old)
        .sadd(old_keys.teams(), new)
        .hset(new_keys.team(), "name", new)
        .hset(new_keys.team(), "updated_at", unix_timestamp_string())
        .query_async(&mut connection)
        .await?;

    Ok(())
}

/// Loads the full membership of a team from a specific Redis instance.
///
/// Reads the per-team agent set, then for each agent fetches its profile hash
/// (type and timestamps) and registration tuples, assembling a sorted list of
/// `TeamMember` values. Agents and registrations are sorted so output is
/// deterministic.
///
/// # Errors
///
/// Returns an error if the Redis connection fails or any of the per-agent reads
/// (hash fields or registration set) fail.
pub async fn list_members_with_url(url: &str, team: &str) -> Result<Vec<TeamMember>> {
    let keys = RedisKeys::new(team);
    let mut connection = redis_client::connect(url).await?;
    let mut agents: Vec<String> = connection.smembers(keys.team_agents()).await?;
    // Sort for stable, alphabetical member ordering across runs.
    agents.sort();

    let mut members = Vec::with_capacity(agents.len());
    for agent in agents {
        let agent_key = keys.agent(&agent);
        // Fetch the three profile fields in a single round trip via a pipeline.
        let (agent_type, created_at, last_seen_at): (String, String, String) = redis::pipe()
            .hget(&agent_key, "type")
            .hget(&agent_key, "created_at")
            .hget(&agent_key, "last_seen_at")
            .query_async(&mut connection)
            .await?;
        let mut registrations: Vec<String> =
            connection.smembers(keys.registrations(&agent)).await?;
        // Sort so the "latest" registration picked below is deterministic.
        registrations.sort();
        // Each registration tuple is "type:project"; take the project portion
        // of the last (highest-sorted) tuple, defaulting to empty when none.
        let parsed_project = registrations
            .last()
            .and_then(|registration| registration.split_once(':').map(|(_, project)| project))
            .unwrap_or("")
            .to_string();
        let project: Option<String> = connection.hget(&agent_key, "project").await?;
        let project = project.unwrap_or(parsed_project);

        members.push(TeamMember {
            name: agent,
            agent_type,
            project,
            created_at,
            last_seen_at,
        });
    }

    Ok(members)
}

/// Registers or refreshes an agent's membership using an existing connection.
///
/// This is the shared core of agent registration. It preserves the agent's
/// original `created_at` if it already exists (treating a re-registration as a
/// heartbeat) while updating `last_seen_at` to now, then applies the membership
/// writes as a single atomic transaction.
///
/// Crate-visible so other modules can register agents on a connection they
/// already hold without re-resolving the Redis URL.
///
/// # Errors
///
/// Returns an error if reading the existing `created_at` field fails or if the
/// registration transaction fails.
pub(crate) async fn register_agent_with_connection(
    connection: &mut redis::aio::MultiplexedConnection,
    keys: &RedisKeys,
    agent: &str,
    agent_type: &str,
    project: &str,
) -> Result<()> {
    let now = unix_timestamp_string();
    let agent_key = keys.agent(agent);

    // Preserve the first-seen timestamp on re-registration: reuse the stored
    // created_at when present, otherwise treat this as the agent's first sight.
    let created_at: Option<String> = connection.hget(&agent_key, "created_at").await?;
    let created_at = created_at.unwrap_or_else(|| now.clone());

    // Apply the membership writes atomically (MULTI/EXEC) so a partial update
    // cannot leave the agent half-registered.
    let mut pipe = redis::pipe();
    pipe.atomic();
    add_agent_registration_to_pipe(
        &mut pipe,
        keys,
        agent,
        agent_type,
        project,
        &created_at,
        &now,
    );

    let _: () = pipe.query_async(connection).await?;

    Ok(())
}

/// Appends the Redis commands that register an agent into `pipe`.
///
/// Queues the writes that add the agent to the team's agent set, populate its
/// profile hash (`name`, `type`, `created_at`, `last_seen_at`), and — when a
/// non-empty `project` is given — record a `type:project` registration tuple.
/// The caller is responsible for executing `pipe` (typically wrapped in
/// `atomic()`), which keeps this helper reusable across both team creation and
/// agent re-registration.
fn add_agent_registration_to_pipe(
    pipe: &mut redis::Pipeline,
    keys: &RedisKeys,
    agent: &str,
    agent_type: &str,
    project: &str,
    created_at: &str,
    last_seen_at: &str,
) {
    let agent_key = keys.agent(agent);
    // Add the agent to the team roster and write/overwrite its profile fields.
    pipe.sadd(keys.team_agents(), agent)
        .hset(&agent_key, "name", agent)
        .hset(&agent_key, "type", agent_type)
        .hset(&agent_key, "project", project)
        .hset(&agent_key, "created_at", created_at)
        .hset(&agent_key, "last_seen_at", last_seen_at);

    // Only record a registration tuple when a project is supplied; the owner's
    // initial registration, for example, has no project.
    if !project.is_empty() {
        pipe.sadd(keys.registrations(agent), format!("{agent_type}:{project}"));
    }
}

fn registration_tuple(agent_type: &str, project: &str) -> String {
    format!("{agent_type}:{project}")
}

async fn registration_for_agent(
    connection: &mut redis::aio::MultiplexedConnection,
    keys: &RedisKeys,
    agent: &str,
    agent_type: &str,
    project: &str,
) -> Result<Option<AgentRegistration>> {
    let tuple = registration_tuple(agent_type, project);
    let exists: bool = connection
        .sismember(keys.registrations(agent), &tuple)
        .await?;
    if !exists {
        return Ok(None);
    }
    let session: Option<String> = connection
        .hget(keys.registration_sessions(agent), &tuple)
        .await?;
    Ok(Some(AgentRegistration {
        agent: agent.to_string(),
        agent_type: agent_type.to_string(),
        project: project.to_string(),
        session,
    }))
}

async fn registrations_for_agent(
    connection: &mut redis::aio::MultiplexedConnection,
    keys: &RedisKeys,
    agent: &str,
) -> Result<Vec<AgentRegistration>> {
    let mut registrations: Vec<String> = connection.smembers(keys.registrations(agent)).await?;
    registrations.sort();
    let sessions: HashMap<String, String> = connection
        .hgetall(keys.registration_sessions(agent))
        .await
        .unwrap_or_default();
    Ok(registrations
        .into_iter()
        .filter_map(|tuple| {
            let (agent_type, project) = tuple.split_once(':')?;
            Some(AgentRegistration {
                agent: agent.to_string(),
                agent_type: agent_type.to_string(),
                project: project.to_string(),
                session: sessions.get(&tuple).cloned(),
            })
        })
        .collect())
}

async fn remove_registration_with_connection(
    connection: &mut redis::aio::MultiplexedConnection,
    keys: &RedisKeys,
    team: &str,
    agent: &str,
    agent_type: &str,
    project: &str,
    session: Option<&str>,
) -> Result<bool> {
    let tuple = registration_tuple(agent_type, project);
    let exists: bool = connection
        .sismember(keys.registrations(agent), &tuple)
        .await?;
    if !exists {
        return Ok(false);
    }
    if let Some(expected_session) = session {
        let actual_session: Option<String> = connection
            .hget(keys.registration_sessions(agent), &tuple)
            .await?;
        if actual_session.as_deref() != Some(expected_session) {
            return Ok(false);
        }
    }

    let _: () = redis::pipe()
        .atomic()
        .srem(keys.registrations(agent), &tuple)
        .hdel(keys.registration_sessions(agent), &tuple)
        .hset(keys.team(), "updated_at", unix_timestamp_string())
        .query_async(connection)
        .await?;

    let remaining: i64 = connection.scard(keys.registrations(agent)).await?;
    if remaining == 0 {
        remove_agent_with_connection(connection, keys, team, agent).await?;
    }

    Ok(true)
}

async fn remove_agent_with_connection(
    connection: &mut redis::aio::MultiplexedConnection,
    keys: &RedisKeys,
    team: &str,
    agent: &str,
) -> Result<()> {
    let removed: i64 = connection.srem(keys.team_agents(), agent).await?;
    if removed == 0 {
        return Err(MagiError::NotFound(format!(
            "agent `{agent}` is not a member of team `{team}`"
        )));
    }
    let _: () = redis::pipe()
        .atomic()
        .del(keys.agent(agent))
        .del(keys.registrations(agent))
        .del(keys.registration_sessions(agent))
        .del(keys.cursor(agent))
        .del(keys.actas_lock(agent))
        .hset(keys.team(), "updated_at", unix_timestamp_string())
        .query_async(connection)
        .await?;
    Ok(())
}

async fn rename_existing_key(
    connection: &mut redis::aio::MultiplexedConnection,
    old: String,
    new: String,
) -> Result<()> {
    let exists: bool = connection.exists(&old).await?;
    if exists {
        let _: () = redis::cmd("RENAME")
            .arg(old)
            .arg(new)
            .query_async(connection)
            .await?;
    }
    Ok(())
}

/// Returns the current time as a Unix-epoch-seconds value rendered as a string.
///
/// Crate-visible so other modules can produce timestamps in the same format
/// used for the `created_at` / `last_seen_at` fields stored in Redis. If the
/// system clock is somehow before the Unix epoch, the duration defaults to zero
/// rather than panicking, yielding `"0"`.
pub(crate) fn unix_timestamp_string() -> String {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .to_string()
}

/// Extracts the configured Redis URL from the loaded application config.
///
/// # Errors
///
/// Returns `MagiError::InvalidConfig` when `redis.url` is absent, signalling
/// that the user has not pointed `magi` at a Redis instance yet.
fn configured_redis_url(config: &AppConfig) -> Result<String> {
    config
        .redis
        .url
        .clone()
        .ok_or_else(|| MagiError::InvalidConfig("redis.url is not configured".to_string()))
}
