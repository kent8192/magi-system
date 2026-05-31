//! Ephemeral, session-scoped agent lifecycle for the `magi` CLI.
//!
//! A Claude Code (or Codex) session is treated as a short-lived agent: it is
//! registered into the active team on session start and removed again on
//! session end. To keep those agents recognizable while still unique, each one
//! is given a codename of the form `<adjective>-<magi>`, where `<magi>` cycles
//! deterministically through the three MAGI deliberation units from
//! *Neon Genesis Evangelion* — Melchior, Balthasar, Casper — and `<adjective>`
//! is drawn at random from a vetted word list.
//!
//! The naming core in this module is deliberately pure and side-effect free so
//! it can be unit-tested without Redis: the cyclic suffix is a function of a
//! monotonic sequence number, and adjective selection and name claiming are
//! injected as closures.

use redis::AsyncCommands;

use crate::config::AppConfig;
use crate::error::{MagiError, Result};
use crate::model::RedisKeys;
use crate::redis_client;
use crate::session_identity::resolve_identity;
use crate::team;

/// Number of `<adjective>-<suffix>` candidates tried before the guaranteed
/// unique `<adjective>-<suffix>-<seq>` fallback.
const MAX_NAME_ATTEMPTS: usize = 16;

/// Agent `type` recorded for session agents spawned without an explicit type.
const DEFAULT_AGENT_TYPE: &str = "claude-code";

/// The three MAGI deliberation units, in the fixed cycle order used for naming.
pub const MAGI_SUFFIXES: [&str; 3] = ["melchior", "balthasar", "casper"];

/// Draws a single random adjective from the bundled `petname` word list.
///
/// The selection uses the caller-supplied RNG (the project's `rand` 0.9) rather
/// than `petname`'s own generator, so `petname`'s independent `rand` version is
/// never coupled into this code path; only its vetted word list is reused.
pub fn random_adjective(rng: &mut impl rand::Rng) -> String {
    use rand::seq::IndexedRandom;

    // `Petnames::default()` exposes the built-in word lists (the `default-words`
    // feature). Pick from the adjectives with our own RNG; fall back to a fixed
    // word only in the impossible case of an empty list.
    petname::Petnames::default()
        .adjectives
        .choose(rng)
        .copied()
        .unwrap_or("magi")
        .to_string()
}

/// Returns the MAGI suffix for a 1-based monotonic sequence number.
///
/// `seq` is expected to come from a Redis `INCR` (which starts at 1), so the
/// first agent is `melchior`, the second `balthasar`, the third `casper`, and
/// the fourth wraps back to `melchior`. A `seq` of 0 is treated as 1.
pub fn magi_suffix(seq: u64) -> &'static str {
    // `seq` is 1-based; map it onto the cycle. `saturating_sub` guards a 0 input
    // (which INCR never produces) so it resolves to the first unit rather than
    // underflowing.
    let index = (seq.saturating_sub(1) % MAGI_SUFFIXES.len() as u64) as usize;
    MAGI_SUFFIXES[index]
}

/// Composes an agent name from an `adjective` and a MAGI `suffix`.
pub fn compose_name(adjective: &str, suffix: &str) -> String {
    format!("{adjective}-{suffix}")
}

/// CLI entry point for `magi agent name`.
///
/// Resolves the current process identity the same way messaging commands do:
/// runtime session records keyed by `MAGI_SESSION_ID`, `CODEX_THREAD_ID`,
/// `CODEX_SESSION_ID`, or `CLAUDE_SESSION_ID`. Runtime session records are the
/// only source for the session agent; the agent name is never read from or
/// written to persistent config.
///
/// # Errors
///
/// Returns `MagiError::InvalidConfig` when no session agent can be resolved.
pub fn name() -> Result<()> {
    let config = AppConfig::load()?;
    let identity = resolve_identity(&config);
    let agent = identity
        .agent
        .ok_or_else(|| MagiError::InvalidConfig("session agent is required".to_string()))?;
    println!("{agent}");
    Ok(())
}

/// Builds the ordered list of candidate names to try for sequence `seq`.
///
/// The first `max_attempts` entries are `<adjective>-<suffix>` names, where
/// `suffix` is fixed by [`magi_suffix`] and each `adjective` is drawn from
/// `next_adjective`. The final entry is a guaranteed-unique fallback,
/// `<adjective>-<suffix>-<seq>`: because `seq` is monotonic, no earlier agent
/// can have produced that exact name.
///
/// The caller claims these names in order (atomically, against Redis) and keeps
/// the first one that was free; the fallback at the end always succeeds. Keeping
/// generation pure (no Redis) lets the cycle, re-roll order, and fallback be
/// unit-tested deterministically by scripting `next_adjective`.
pub fn candidate_names(
    seq: u64,
    max_attempts: usize,
    mut next_adjective: impl FnMut() -> String,
) -> Vec<String> {
    let suffix = magi_suffix(seq);
    let mut names = Vec::with_capacity(max_attempts + 1);
    for _ in 0..max_attempts {
        names.push(compose_name(&next_adjective(), suffix));
    }
    // Guaranteed-unique fallback: appending the monotonic `seq` means this name
    // cannot collide with any earlier agent's, so a claim of it always succeeds.
    names.push(format!("{}-{seq}", compose_name(&next_adjective(), suffix)));
    names
}

/// Returns the Redis key holding a team's monotonic agent sequence counter.
///
/// The counter is `INCR`-emented once per spawn and drives the deterministic
/// MAGI suffix cycle; it is intentionally never decremented, so the cycle keeps
/// advancing even as agents come and go.
fn agent_seq_key(keys: &RedisKeys) -> String {
    format!("{}:agent_seq", keys.team())
}

/// Spawns a new ephemeral agent into `team` on the Redis at `url`.
///
/// Increments the team's monotonic sequence counter to pick the deterministic
/// MAGI suffix, then claims a unique `<adjective>-<suffix>` name by atomically
/// `SADD`-ing candidates until one is free (falling back to a seq-suffixed name
/// that cannot collide). The claimed agent is registered with `agent_type` and
/// `project`, mirroring the membership writes used by invite-based joins.
///
/// Returns the assigned name.
///
/// # Errors
///
/// Propagates Redis connection/command errors and any registration failure.
pub async fn spawn_with_url(
    url: &str,
    team: &str,
    agent_type: &str,
    project: &str,
    rng: &mut impl rand::Rng,
) -> Result<String> {
    let keys = RedisKeys::new(team);
    let mut connection = redis_client::connect(url).await?;

    // Monotonic per-team counter drives the deterministic MAGI suffix cycle.
    let seq: i64 = connection.incr(agent_seq_key(&keys), 1_i64).await?;
    let seq = seq.max(1) as u64;

    // Try each candidate in order; `SADD` returning 1 is the atomic claim of a
    // free name. The candidate list always ends with a seq-suffixed fallback
    // that cannot collide, so the loop is guaranteed to claim a name.
    for name in candidate_names(seq, MAX_NAME_ATTEMPTS, || random_adjective(&mut *rng)) {
        let added: i64 = connection.sadd(keys.team_agents(), &name).await?;
        if added == 1 {
            team::register_agent_with_connection(
                &mut connection,
                &keys,
                &name,
                agent_type,
                project,
            )
            .await?;
            return Ok(name);
        }
    }

    // Unreachable: the fallback candidate is always free.
    Err(MagiError::InvalidConfig(
        "could not assign a unique agent name".to_string(),
    ))
}

/// Removes an ephemeral agent named `agent` from `team` on the Redis at `url`.
///
/// Drops the roster entry, profile hash, registration tuples, and inbox cursor,
/// and bumps the team's `updated_at`. Message stream history is intentionally
/// left intact as the team's durable audit trail.
///
/// # Errors
///
/// Returns `MagiError::NotFound` when the agent is not a member of the team, and
/// propagates Redis connection/command errors.
pub async fn despawn_with_url(url: &str, team: &str, agent: &str) -> Result<()> {
    let keys = RedisKeys::new(team);
    let mut connection = redis_client::connect(url).await?;

    // SREM reports how many members were removed; 0 means the agent was not in
    // the roster, which we surface as a not-found error.
    let removed: i64 = connection.srem(keys.team_agents(), agent).await?;
    if removed == 0 {
        return Err(MagiError::NotFound(format!(
            "agent `{agent}` is not a member of team `{team}`"
        )));
    }

    // Clean up the agent's scoped keys atomically and record the roster change.
    let now = team::unix_timestamp_string();
    let _: () = redis::pipe()
        .atomic()
        .del(keys.agent(agent))
        .del(keys.registrations(agent))
        .del(keys.cursor(agent))
        .hset(keys.team(), "updated_at", now)
        .query_async(&mut connection)
        .await?;

    Ok(())
}

/// Extracts the configured Redis URL, erroring when it is unset.
fn configured_redis_url(config: &AppConfig) -> Result<String> {
    config
        .redis
        .url
        .clone()
        .ok_or_else(|| MagiError::InvalidConfig("redis.url is not configured".to_string()))
}

/// CLI entry point for `magi agent spawn`.
///
/// Resolves the team (explicit `team` or the active team), spawns a uniquely
/// named ephemeral agent and prints the assigned name on its own line for hooks
/// to capture in the per-session record.
///
/// # Errors
///
/// Returns `MagiError::InvalidConfig` when neither a team argument nor an active
/// team is available, and propagates config, Redis, and registration errors.
pub async fn spawn(team: Option<String>, agent_type: Option<String>) -> Result<()> {
    let config = AppConfig::load()?;
    let url = configured_redis_url(&config)?;
    let team = team
        .or_else(|| config.identity.active_team.clone())
        .ok_or_else(|| MagiError::InvalidConfig("identity.active_team is required".to_string()))?;
    let agent_type = agent_type.unwrap_or_else(|| DEFAULT_AGENT_TYPE.to_string());
    let project = std::env::current_dir()?.display().to_string();

    let mut rng = rand::rng();
    let name = spawn_with_url(&url, &team, &agent_type, &project, &mut rng).await?;

    // Print only the name so callers (e.g. the SessionStart hook) can capture it.
    println!("{name}");
    Ok(())
}

/// CLI entry point for `magi agent despawn`.
///
/// Resolves the team and the agent name (from the argument or the current
/// session identity) and removes the agent. Removal is idempotent: despawning
/// an agent that is already gone succeeds, so a repeated SessionEnd hook never
/// fails.
///
/// # Errors
///
/// Returns `MagiError::InvalidConfig` when the team or name cannot be resolved,
/// and propagates config and Redis errors other than a missing member.
pub async fn despawn(team: Option<String>, name: Option<String>) -> Result<()> {
    let config = AppConfig::load()?;
    let url = configured_redis_url(&config)?;
    let identity = resolve_identity(&config);
    let team = team
        .or(identity.team)
        .ok_or_else(|| MagiError::InvalidConfig("identity.active_team is required".to_string()))?;
    let name = name
        .or(identity.agent)
        .ok_or_else(|| MagiError::InvalidConfig("agent name is required".to_string()))?;

    match despawn_with_url(&url, &team, &name).await {
        Ok(()) => println!("Despawned {name} from team: {team}"),
        // Idempotent: an already-absent agent is a no-op success.
        Err(MagiError::NotFound(_)) => println!("Agent {name} is not a member of team: {team}"),
        Err(error) => return Err(error),
    }
    Ok(())
}
