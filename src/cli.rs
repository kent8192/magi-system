//! Command-line interface definition for the `magi` CLI tool.
//!
//! This module uses `clap` to declare every subcommand, flag, and argument
//! that `magi` accepts.  It is purely declarative — no business logic lives
//! here.  After `clap` parses `argv`, the resulting `Cli` value is handed
//! off to `main` (or a dispatcher in `lib.rs`) which routes each `Command`
//! variant to the appropriate handler module (Redis lifecycle, messaging,
//! team management, SSH helpers, installer, etc.).
//!
//! # Top-level structure
//!
//! ```text
//! magi [SUBCOMMAND]
//!   redis   {start|status|stop}
//!   team    {create|list|members}
//!   invite  {create|list|revoke}
//!   join    --invite <TOKEN>
//!   send    <TO> <MESSAGE>...
//!   inbox
//!   history [--team <T>] [--agent <A>] [--limit <N>]
//!   watch   [--format line|json|context] [--once]
//!   codex   {bridge}
//!   ssh     {start|status|stop}
//!   install
//!   config  {get|set}
//! ```
//!
//! When invoked with no subcommand, `magi` falls back to an interactive REPL
//! (handled by the caller when `Cli::command` is `None`).

use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

/// Root CLI struct parsed from `argv` by `clap`.
///
/// `command` is `None` when the user runs `magi` with no subcommand, which
/// signals the caller to enter interactive REPL mode.
#[derive(Debug, Parser)]
#[command(name = "magi", version, about = "Redis-backed agent messaging")]
pub struct Cli {
    /// The subcommand to execute, or `None` to enter interactive REPL mode.
    #[command(subcommand)]
    pub command: Option<Command>,
}

/// All top-level subcommands supported by `magi`.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Manage the embedded (managed) Redis server lifecycle.
    ///
    /// The managed Redis process is owned by `magi` and stores its PID and
    /// socket information under `~/.magi/redis/`.  Use these subcommands to
    /// start, query, or stop it without needing a separately installed Redis.
    Redis {
        #[command(subcommand)]
        command: RedisCommand,
    },

    /// Manage agent teams (create teams, list them, view members).
    Team {
        #[command(subcommand)]
        command: TeamCommand,
    },

    /// Manage invite tokens used for invite-based onboarding.
    ///
    /// Invite tokens are stored in Redis and carry an expiry (`--ttl`).
    /// A remote agent redeems a token via the `join` subcommand.
    Invite {
        #[command(subcommand)]
        command: InviteCommand,
    },

    /// Redeem an invite token to join a team.
    ///
    /// This is the counterpart to `invite create`.  The `--invite` argument
    /// must be a valid, non-expired token produced by `magi invite create`.
    Join {
        /// The invite token to redeem (produced by `magi invite create`).
        #[arg(long)]
        invite: String,
    },

    /// Manage ephemeral, session-scoped agents.
    ///
    /// `name` prints the current session-aware agent name. `spawn` registers a
    /// uniquely named agent (a `<adjective>-<magi>` codename) into the active
    /// team; `despawn` removes it again.
    /// Intended to be driven by a session lifecycle hook.
    Agent {
        #[command(subcommand)]
        command: AgentCommand,
    },

    /// Manage explicit project/type registrations.
    Registration {
        #[command(subcommand)]
        command: RegistrationCommand,
    },

    /// Discover project/type identities without persistent active-agent state.
    Identity {
        #[command(subcommand)]
        command: IdentityCommand,
    },

    /// Manage actas-style exclusive role claims.
    Actas {
        #[command(subcommand)]
        command: ActasCommand,
    },

    /// Manage delivery mode configuration for runtime integrations.
    Delivery {
        #[command(subcommand)]
        command: DeliveryCommand,
    },

    /// Send a message to another agent or team via Redis Streams.
    ///
    /// `to` identifies the recipient (agent name or team name).  All
    /// remaining positional arguments are joined into the message body,
    /// allowing callers to omit shell quoting for simple messages.
    Send {
        /// Recipient agent or team name.
        to: String,
        /// Message words; multiple tokens are joined with spaces before delivery.
        #[arg(required = true, num_args = 1.., trailing_var_arg = true)]
        message: Vec<String>,
    },

    /// Display unread messages in an inbox.
    Inbox {
        /// Team to read from; defaults to the active team.
        #[arg(long)]
        team: Option<String>,
        /// Agent to read for; defaults to the current session agent.
        #[arg(long)]
        agent: Option<String>,
        /// Suppress no-message output for hook use.
        #[arg(long)]
        quiet: bool,
        /// Hook-oriented output format.
        #[arg(long, value_enum)]
        hook_format: Option<HookFormat>,
    },

    /// Display message history, optionally filtered by team or agent.
    ///
    /// Without filters the full stream history visible to this agent is shown.
    History {
        /// Restrict output to messages belonging to this team.
        #[arg(long)]
        team: Option<String>,
        /// Restrict output to messages sent by or to this agent.
        #[arg(long)]
        agent: Option<String>,
        /// Maximum number of messages to print.
        #[arg(long)]
        limit: Option<usize>,
    },

    /// Subscribe to the Redis Pub/Sub channel and stream incoming messages.
    ///
    /// Runs until interrupted (Ctrl-C) unless `--once` is set.  Use
    /// `--format json` to emit newline-delimited JSON suitable for machine
    /// consumption; `context` emits agent-context lines; the default `line`
    /// format is human-readable.
    Watch {
        /// Output format: `line` (human-readable), `json` (NDJSON), or
        /// `context` (`from->to: body`).
        #[arg(long, value_enum, default_value_t = WatchFormat::Line)]
        format: WatchFormat,
        /// Exit after the first non-empty delivery batch.
        #[arg(long)]
        once: bool,
    },

    /// Integrate magi delivery with Codex-specific runtime surfaces.
    Codex {
        #[command(subcommand)]
        command: CodexCommand,
    },

    /// Manage the SSH helper process used for secure remote connections.
    ///
    /// The SSH helper facilitates agent-to-agent communication across hosts.
    /// Its lifecycle (PID file, port) is tracked under `~/.magi/ssh/`.
    Ssh {
        #[command(subcommand)]
        command: SshCommand,
    },

    /// Install `magi` binaries and set up the `~/.magi` state directory.
    ///
    /// Places the primary binary at `~/.agents/skills/magi/bin/magi` and
    /// creates a symlink at `~/.local/bin/magi` so the command is on `PATH`.
    Install,

    /// Read or write persistent `magi` configuration values.
    ///
    /// Configuration is stored as key-value pairs under `~/.magi/config`.
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
}

/// Subcommands for managing the embedded Redis server.
#[derive(Debug, Subcommand)]
pub enum RedisCommand {
    /// Start the managed Redis server.
    ///
    /// By default Redis binds only to `127.0.0.1`.  Pass `--lan` to also
    /// bind on the LAN interface, or `--bind` to specify an address explicitly.
    Start {
        /// Bind on the LAN interface in addition to loopback.
        #[arg(long)]
        lan: bool,
        /// Explicit bind address (overrides `--lan`).
        #[arg(long)]
        bind: Option<String>,
    },
    /// Report whether the managed Redis server is running and print its address.
    Status,
    /// Stop the managed Redis server gracefully.
    Stop,
    /// Stop managed Redis, clear its persisted data, and start it again.
    Reset,
}

/// Subcommands for managing the SSH helper process.
#[derive(Debug, Subcommand)]
pub enum SshCommand {
    /// Start the SSH helper process.
    Start,
    /// Report the SSH helper's running status and listening port.
    Status,
    /// Stop the SSH helper process.
    Stop,
}

/// Subcommands for team management.
#[derive(Debug, Subcommand)]
pub enum TeamCommand {
    /// Create a new team with the given name.
    Create {
        /// Unique name for the new team.
        name: String,
    },
    /// List all teams the current agent belongs to.
    List,
    /// List the members of a team.
    ///
    /// If `--team` is omitted, the default team for this agent is used.
    Members {
        /// Name of the team whose members to list.
        #[arg(long)]
        team: Option<String>,
    },
    /// Rename a team.
    Rename {
        /// Current team name.
        old: String,
        /// New team name.
        new: String,
    },
}

/// Subcommands for ephemeral session-scoped agent management.
#[derive(Debug, Subcommand)]
pub enum AgentCommand {
    /// Print the current agent name.
    ///
    /// Session records are the source of truth for the current MAGI agent, so
    /// concurrent sessions report their own names.
    Name,
    /// Spawn a uniquely named ephemeral agent into the active team.
    ///
    /// Assigns the next `<adjective>-<magi>` codename, registers it, and prints
    /// the assigned name.
    Spawn {
        /// Team to join; defaults to the configured active team when omitted.
        #[arg(long)]
        team: Option<String>,
        /// Agent type recorded in the profile (defaults to `claude-code`).
        #[arg(long = "type")]
        agent_type: Option<String>,
    },
    /// Remove an ephemeral agent from a team.
    ///
    /// Defaults the team to the active team and the name to the session agent.
    /// Removing an agent that is already gone is treated as success.
    Despawn {
        /// Team to remove from; defaults to the configured active team.
        #[arg(long)]
        team: Option<String>,
        /// Agent name to remove; defaults to the current session agent.
        #[arg(long)]
        name: Option<String>,
    },
    /// Rename an agent within a team.
    Rename {
        /// Team containing the agent.
        #[arg(long)]
        team: String,
        /// Current agent name.
        old: String,
        /// New agent name.
        new: String,
    },
}

/// Subcommands for direct registration management.
#[derive(Debug, Subcommand)]
pub enum RegistrationCommand {
    /// Add or refresh a project/type registration.
    Add {
        #[arg(long)]
        team: String,
        #[arg(long)]
        agent: String,
        #[arg(long = "type")]
        agent_type: String,
        #[arg(long)]
        project: PathBuf,
        #[arg(long)]
        session: Option<String>,
    },
    /// Remove an agent and all of its registrations.
    Remove {
        #[arg(long)]
        team: String,
        #[arg(long)]
        agent: String,
    },
    /// Remove registrations matching project/type filters.
    Reset {
        #[arg(long)]
        project: PathBuf,
        #[arg(long = "type")]
        agent_type: String,
        #[arg(long)]
        agent: Option<String>,
        #[arg(long)]
        session: Option<String>,
    },
}

/// Subcommands for project/type identity discovery.
#[derive(Debug, Subcommand)]
pub enum IdentityCommand {
    /// List identities registered for the project/type pair.
    List {
        #[arg(long)]
        project: PathBuf,
        #[arg(long = "type")]
        agent_type: String,
    },
    /// Resolve the current identity for a project/type pair.
    Whoami {
        #[arg(long)]
        project: PathBuf,
        #[arg(long = "type")]
        agent_type: String,
    },
}

/// Subcommands for actas-style exclusive role claims.
#[derive(Debug, Subcommand)]
pub enum ActasCommand {
    /// Claim exclusive use of an agent role for a session.
    Claim {
        agent: String,
        #[arg(long)]
        team: Option<String>,
        #[arg(long)]
        session: Option<String>,
        #[arg(long, default_value_t = 3600)]
        ttl: u64,
    },
    /// Release an exclusive role claim.
    Release {
        agent: String,
        #[arg(long)]
        team: Option<String>,
        #[arg(long)]
        session: Option<String>,
    },
    /// Print current claim status for a role.
    Status {
        agent: String,
        #[arg(long)]
        team: Option<String>,
    },
    /// Garbage-collect stale claims.
    Gc,
}

/// Subcommands for runtime delivery mode configuration.
#[derive(Debug, Subcommand)]
pub enum DeliveryCommand {
    /// Set delivery mode for a project/type pair.
    Set {
        mode: DeliveryMode,
        #[arg(long = "type")]
        agent_type: String,
        #[arg(long)]
        project: PathBuf,
    },
    /// Show delivery mode for a project/type pair.
    Status {
        #[arg(long = "type")]
        agent_type: Option<String>,
        #[arg(long)]
        project: Option<PathBuf>,
    },
    /// Refresh delivery configuration for a project/type pair.
    Restart {
        #[arg(long = "type")]
        agent_type: String,
        #[arg(long)]
        project: PathBuf,
    },
    /// Disable delivery for a project/type pair.
    Stop {
        #[arg(long = "type")]
        agent_type: String,
        #[arg(long)]
        project: PathBuf,
    },
}

/// Subcommands for invite-token management.
#[derive(Debug, Subcommand)]
pub enum InviteCommand {
    /// Create a new invite token for a team.
    ///
    /// The token is stored in Redis with the specified TTL and printed to
    /// stdout so it can be shared with the agent being invited.
    Create {
        /// The team the invitee will join upon redemption.
        #[arg(long)]
        team: String,
        /// Time-to-live for the invite token (e.g. `"24h"`, `"7d"`).
        #[arg(long, default_value = "24h")]
        ttl: String,
    },
    /// List active (non-expired) invite tokens for a team.
    List {
        /// Team whose invite tokens to list.
        #[arg(long)]
        team: String,
    },
    /// Revoke an invite token immediately, regardless of its remaining TTL.
    Revoke {
        /// The ID of the invite token to revoke.
        invite_id: String,
    },
}

/// Subcommands for reading or writing persistent configuration.
#[derive(Debug, Subcommand)]
pub enum ConfigCommand {
    /// Read a configuration value by key.
    Get { key: String },
    /// Write a configuration value.
    Set { key: String, value: String },
    /// Print the full configuration document.
    Show,
}

/// Hook-oriented inbox output format.
#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum HookFormat {
    /// JSON payload suitable for Codex hooks.
    Codex,
    /// JSON payload suitable for Claude Code hooks.
    ClaudeCode,
}

/// Delivery mode values.
#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum DeliveryMode {
    /// Monitor-based live delivery.
    Monitor,
    /// Turn-based delivery.
    Turn,
    /// Both monitor and turn delivery.
    Both,
    /// Delivery disabled.
    Off,
}

/// Subcommands for Codex runtime integrations.
#[derive(Debug, Subcommand)]
pub enum CodexCommand {
    /// Bridge Redis Pub/Sub delivery into a running Codex app-server thread.
    Bridge {
        /// Codex thread id to inject into; defaults to CODEX_THREAD_ID/CODEX_SESSION_ID.
        #[arg(long)]
        thread: Option<String>,
        /// Working directory to pass to new app-server turns.
        #[arg(long)]
        cwd: Option<std::path::PathBuf>,
        /// Codex CLI executable used by hook-managed daemon startup.
        #[arg(long, default_value = "codex")]
        codex: String,
        /// Codex app-server Unix control socket used for WebSocket delivery.
        #[arg(long)]
        socket: Option<std::path::PathBuf>,
    },
}

/// Output format used by the `watch` subcommand.
#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum WatchFormat {
    /// Human-readable one-message-per-line format (default).
    Line,
    /// Newline-delimited JSON (NDJSON) for machine consumption.
    Json,
    /// Agent-context format: `<from>-><to>: <body>`.
    Context,
}
