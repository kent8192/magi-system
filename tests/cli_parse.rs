//! CLI argument-parsing tests for `magi`.
//!
//! These tests exercise [`magi::cli::Cli`] via [`clap::Parser::try_parse_from`], verifying that
//! every subcommand and its flags are wired up correctly.  No Redis server is required; the suite
//! is purely in-process and runs offline.
//!
//! Coverage areas:
//! - No-subcommand invocation (falls through to interactive REPL mode).
//! - `redis` subcommands: `start` (with and without `--lan`/`--bind`), `status`, `stop`, `reset`.
//! - `invite` subcommands: `create` (explicit and default TTL), `list`, `revoke`.
//! - `team` subcommands: `create`, `list`, `members` (with and without `--team` filter).
//! - `send` (multi-word message tail, missing-message rejection).
//! - `join` (with `--invite`, missing-invite rejection).
//! - `history` (with and without optional `--team`/`--agent` filters).
//! - `inbox`, `watch` (line/json/context format, `--once`, invalid-format rejection).
//! - `config get`/`config set`.
//! - `install`, `ssh start`/`status`/`stop`.

use clap::Parser;
use magi::cli::{
    ActasCommand, AgentCommand, Cli, CodexCommand, Command, ConfigCommand, DeliveryCommand,
    DeliveryMode, HookFormat, IdentityCommand, InviteCommand, RedisCommand, RegistrationCommand,
    SshCommand, TeamCommand, WatchFormat,
};

#[test]
fn parses_default_interactive_mode() {
    let cli = Cli::try_parse_from(["magi"]).expect("parse");
    assert!(cli.command.is_none());
}

#[test]
fn parses_redis_start_lan_bind() {
    let cli = Cli::try_parse_from(["magi", "redis", "start", "--lan", "--bind", "0.0.0.0"])
        .expect("parse");

    let Some(Command::Redis {
        command: RedisCommand::Start { lan, bind },
    }) = cli.command
    else {
        panic!("expected redis start");
    };

    assert!(lan);
    assert_eq!(bind.as_deref(), Some("0.0.0.0"));
}

#[test]
fn parses_redis_start_defaults() {
    let cli = Cli::try_parse_from(["magi", "redis", "start"]).expect("parse");

    let Some(Command::Redis {
        command: RedisCommand::Start { lan, bind },
    }) = cli.command
    else {
        panic!("expected redis start");
    };

    assert!(!lan);
    assert_eq!(bind, None);
}

#[test]
fn parses_redis_status() {
    let cli = Cli::try_parse_from(["magi", "redis", "status"]).expect("parse");

    let Some(Command::Redis {
        command: RedisCommand::Status,
    }) = cli.command
    else {
        panic!("expected redis status");
    };
}

#[test]
fn parses_redis_stop() {
    let cli = Cli::try_parse_from(["magi", "redis", "stop"]).expect("parse");

    let Some(Command::Redis {
        command: RedisCommand::Stop,
    }) = cli.command
    else {
        panic!("expected redis stop");
    };
}

#[test]
fn parses_redis_reset() {
    let cli = Cli::try_parse_from(["magi", "redis", "reset"]).expect("parse");

    let Some(Command::Redis {
        command: RedisCommand::Reset,
    }) = cli.command
    else {
        panic!("expected redis reset");
    };
}

#[test]
fn parses_invite_create_with_ttl() {
    let cli = Cli::try_parse_from(["magi", "invite", "create", "--team", "core", "--ttl", "24h"])
        .expect("parse");

    let Some(Command::Invite {
        command: InviteCommand::Create { team, ttl },
    }) = cli.command
    else {
        panic!("expected invite create");
    };

    assert_eq!(team, "core");
    assert_eq!(ttl, "24h");
}

#[test]
fn parses_invite_create_default_ttl() {
    // When `--ttl` is omitted the clap default must be `"24h"`.
    let cli = Cli::try_parse_from(["magi", "invite", "create", "--team", "core"]).expect("parse");

    let Some(Command::Invite {
        command: InviteCommand::Create { team, ttl },
    }) = cli.command
    else {
        panic!("expected invite create");
    };

    assert_eq!(team, "core");
    assert_eq!(ttl, "24h");
}

#[test]
fn parses_invite_list_requires_team() {
    let cli = Cli::try_parse_from(["magi", "invite", "list", "--team", "core"]).expect("parse");

    let Some(Command::Invite {
        command: InviteCommand::List { team },
    }) = cli.command
    else {
        panic!("expected invite list");
    };

    assert_eq!(team, "core");
}

#[test]
fn rejects_invite_list_without_team() {
    // `--team` is required for `invite list`; omitting it must produce a parse error.
    let error = Cli::try_parse_from(["magi", "invite", "list"]);
    assert!(error.is_err());
}

#[test]
fn parses_invite_revoke_invite_id() {
    let cli = Cli::try_parse_from(["magi", "invite", "revoke", "inv_123"]).expect("parse");

    let Some(Command::Invite {
        command: InviteCommand::Revoke { invite_id },
    }) = cli.command
    else {
        panic!("expected invite revoke");
    };

    assert_eq!(invite_id, "inv_123");
}

#[test]
fn parses_team_create() {
    let cli = Cli::try_parse_from(["magi", "team", "create", "core"]).expect("parse");
    let Some(Command::Team {
        command: TeamCommand::Create { name },
    }) = cli.command
    else {
        panic!("expected team create");
    };

    assert_eq!(name, "core");
}

#[test]
fn parses_team_list() {
    let cli = Cli::try_parse_from(["magi", "team", "list"]).expect("parse");
    let Some(Command::Team {
        command: TeamCommand::List,
    }) = cli.command
    else {
        panic!("expected team list");
    };
}

#[test]
fn parses_team_members_with_team_filter() {
    let cli = Cli::try_parse_from(["magi", "team", "members", "--team", "core"]).expect("parse");
    let Some(Command::Team {
        command: TeamCommand::Members { team },
    }) = cli.command
    else {
        panic!("expected team members");
    };

    assert_eq!(team.as_deref(), Some("core"));
}

#[test]
fn parses_team_members_without_team_filter() {
    let cli = Cli::try_parse_from(["magi", "team", "members"]).expect("parse");
    let Some(Command::Team {
        command: TeamCommand::Members { team },
    }) = cli.command
    else {
        panic!("expected team members");
    };

    assert_eq!(team, None);
}

#[test]
fn parses_send_message_tail() {
    // The message body is a variadic trailing argument: all words after the recipient
    // are collected into the `message` Vec in order.
    let cli = Cli::try_parse_from(["magi", "send", "bob", "deploy", "is", "done"]).expect("parse");
    let Some(Command::Send { to, message }) = cli.command else {
        panic!("expected send");
    };

    assert_eq!(to, "bob");
    assert_eq!(message, vec!["deploy", "is", "done"]);
}

#[test]
fn rejects_send_without_message_word() {
    // At least one message word is required; a bare `send <to>` must be rejected.
    let error = Cli::try_parse_from(["magi", "send", "bob"]);
    assert!(error.is_err());
}

#[test]
fn parses_join_with_invite() {
    let cli = Cli::try_parse_from(["magi", "join", "--invite", "invite-token"]).expect("parse");
    let Some(Command::Join { invite }) = cli.command else {
        panic!("expected join");
    };

    assert_eq!(invite, "invite-token");
}

#[test]
fn rejects_join_without_invite() {
    // `--invite` is mandatory for the `join` subcommand.
    let error = Cli::try_parse_from(["magi", "join"]);
    assert!(error.is_err());
}

#[test]
fn parses_history_filters() {
    let cli = Cli::try_parse_from(["magi", "history", "--team", "core", "--agent", "alice"])
        .expect("parse");
    let Some(Command::History { team, agent, limit }) = cli.command else {
        panic!("expected history");
    };

    assert_eq!(team.as_deref(), Some("core"));
    assert_eq!(agent.as_deref(), Some("alice"));
    assert_eq!(limit, None);
}

#[test]
fn parses_history_limit() {
    let cli = Cli::try_parse_from(["magi", "history", "--limit", "3"]).expect("parse");
    let Some(Command::History { team, agent, limit }) = cli.command else {
        panic!("expected history");
    };

    assert_eq!(team, None);
    assert_eq!(agent, None);
    assert_eq!(limit, Some(3));
}

#[test]
fn parses_inbox_defaults() {
    let cli = Cli::try_parse_from(["magi", "inbox"]).expect("parse");
    let Some(Command::Inbox {
        team,
        agent,
        quiet,
        hook_format,
    }) = cli.command
    else {
        panic!("expected inbox");
    };
    assert_eq!(team, None);
    assert_eq!(agent, None);
    assert!(!quiet);
    assert_eq!(hook_format, None);
}

#[test]
fn parses_inbox_explicit_hook_options() {
    let cli = Cli::try_parse_from([
        "magi",
        "inbox",
        "--team",
        "core",
        "--agent",
        "alice",
        "--quiet",
        "--hook-format",
        "codex",
    ])
    .expect("parse");
    let Some(Command::Inbox {
        team,
        agent,
        quiet,
        hook_format,
    }) = cli.command
    else {
        panic!("expected inbox");
    };
    assert_eq!(team.as_deref(), Some("core"));
    assert_eq!(agent.as_deref(), Some("alice"));
    assert!(quiet);
    assert_eq!(hook_format, Some(HookFormat::Codex));
}

#[test]
fn parses_watch_default_line_format() {
    let cli = Cli::try_parse_from(["magi", "watch"]).expect("parse");
    let Some(Command::Watch { format, once }) = cli.command else {
        panic!("expected watch");
    };

    assert_eq!(format, WatchFormat::Line);
    assert!(!once);
}

#[test]
fn parses_watch_json_format() {
    let cli = Cli::try_parse_from(["magi", "watch", "--format", "json"]).expect("parse");
    let Some(Command::Watch { format, once }) = cli.command else {
        panic!("expected watch");
    };

    assert_eq!(format, WatchFormat::Json);
    assert!(!once);
}

#[test]
fn parses_watch_context_format_once() {
    let cli =
        Cli::try_parse_from(["magi", "watch", "--format", "context", "--once"]).expect("parse");
    let Some(Command::Watch { format, once }) = cli.command else {
        panic!("expected watch");
    };

    assert_eq!(format, WatchFormat::Context);
    assert!(once);
}

#[test]
fn rejects_invalid_watch_format() {
    // Only documented formats are accepted by `WatchFormat`; any other value must fail.
    let error = Cli::try_parse_from(["magi", "watch", "--format", "xml"]);
    assert!(error.is_err());
}

#[test]
fn parses_codex_bridge_defaults() {
    let cli = Cli::try_parse_from(["magi", "codex", "bridge"]).expect("parse");
    let Some(Command::Codex {
        command:
            CodexCommand::Bridge {
                thread,
                cwd,
                codex,
                socket,
            },
    }) = cli.command
    else {
        panic!("expected codex bridge");
    };

    assert_eq!(thread, None);
    assert_eq!(cwd, None);
    assert_eq!(codex, "codex");
    assert_eq!(socket, None);
}

#[test]
fn parses_codex_bridge_overrides() {
    let cli = Cli::try_parse_from([
        "magi",
        "codex",
        "bridge",
        "--thread",
        "thread-123",
        "--cwd",
        "/tmp/project",
        "--codex",
        "/tmp/codex",
        "--socket",
        "/tmp/codex.sock",
    ])
    .expect("parse");
    let Some(Command::Codex {
        command:
            CodexCommand::Bridge {
                thread,
                cwd,
                codex,
                socket,
            },
    }) = cli.command
    else {
        panic!("expected codex bridge");
    };

    assert_eq!(thread.as_deref(), Some("thread-123"));
    assert_eq!(cwd.as_deref(), Some(std::path::Path::new("/tmp/project")));
    assert_eq!(codex, "/tmp/codex");
    assert_eq!(
        socket.as_deref(),
        Some(std::path::Path::new("/tmp/codex.sock"))
    );
}

#[test]
fn parses_config_get() {
    let cli = Cli::try_parse_from(["magi", "config", "get", "redis.port"]).expect("parse");
    let Some(Command::Config {
        command: ConfigCommand::Get { key },
    }) = cli.command
    else {
        panic!("expected config get");
    };

    assert_eq!(key, "redis.port");
}

#[test]
fn parses_config_set() {
    let cli = Cli::try_parse_from(["magi", "config", "set", "redis.port", "6380"]).expect("parse");
    let Some(Command::Config {
        command: ConfigCommand::Set { key, value },
    }) = cli.command
    else {
        panic!("expected config set");
    };

    assert_eq!(key, "redis.port");
    assert_eq!(value, "6380");
}

#[test]
fn parses_config_show() {
    let cli = Cli::try_parse_from(["magi", "config", "show"]).expect("parse");
    let Some(Command::Config {
        command: ConfigCommand::Show,
    }) = cli.command
    else {
        panic!("expected config show");
    };
}

#[test]
fn parses_install_command() {
    let cli = Cli::try_parse_from(["magi", "install"]).expect("parse");
    let Some(Command::Install) = cli.command else {
        panic!("expected install");
    };
}

#[test]
fn parses_ssh_start() {
    let cli = Cli::try_parse_from(["magi", "ssh", "start"]).expect("parse");
    let Some(Command::Ssh {
        command: SshCommand::Start,
    }) = cli.command
    else {
        panic!("expected ssh start");
    };
}

#[test]
fn parses_ssh_status() {
    let cli = Cli::try_parse_from(["magi", "ssh", "status"]).expect("parse");
    let Some(Command::Ssh {
        command: SshCommand::Status,
    }) = cli.command
    else {
        panic!("expected ssh status");
    };
}

#[test]
fn parses_ssh_stop() {
    let cli = Cli::try_parse_from(["magi", "ssh", "stop"]).expect("parse");
    let Some(Command::Ssh {
        command: SshCommand::Stop,
    }) = cli.command
    else {
        panic!("expected ssh stop");
    };
}

// --- `agent` subcommand parsing ---

#[test]
fn parses_agent_spawn_with_team_and_type() {
    let cli = Cli::try_parse_from([
        "magi", "agent", "spawn", "--team", "core", "--type", "codex",
    ])
    .expect("parse");

    let Some(Command::Agent {
        command: AgentCommand::Spawn { team, agent_type },
    }) = cli.command
    else {
        panic!("expected agent spawn");
    };
    assert_eq!(team.as_deref(), Some("core"));
    assert_eq!(agent_type.as_deref(), Some("codex"));
}

#[test]
fn parses_agent_spawn_defaults_to_none() {
    let cli = Cli::try_parse_from(["magi", "agent", "spawn"]).expect("parse");

    let Some(Command::Agent {
        command: AgentCommand::Spawn { team, agent_type },
    }) = cli.command
    else {
        panic!("expected agent spawn");
    };
    assert!(team.is_none());
    assert!(agent_type.is_none());
}

#[test]
fn parses_agent_name() {
    let cli = Cli::try_parse_from(["magi", "agent", "name"]).expect("parse");
    let Some(Command::Agent {
        command: AgentCommand::Name,
    }) = cli.command
    else {
        panic!("expected agent name");
    };
}

#[test]
fn parses_agent_despawn_with_team_and_name() {
    let cli = Cli::try_parse_from([
        "magi",
        "agent",
        "despawn",
        "--team",
        "core",
        "--name",
        "quiet-melchior",
    ])
    .expect("parse");

    let Some(Command::Agent {
        command: AgentCommand::Despawn { team, name },
    }) = cli.command
    else {
        panic!("expected agent despawn");
    };
    assert_eq!(team.as_deref(), Some("core"));
    assert_eq!(name.as_deref(), Some("quiet-melchior"));
}

#[test]
fn parses_agent_rename() {
    let cli = Cli::try_parse_from(["magi", "agent", "rename", "--team", "core", "old", "new"])
        .expect("parse");
    let Some(Command::Agent {
        command: AgentCommand::Rename { team, old, new },
    }) = cli.command
    else {
        panic!("expected agent rename");
    };
    assert_eq!(team, "core");
    assert_eq!(old, "old");
    assert_eq!(new, "new");
}

#[test]
fn parses_team_rename() {
    let cli = Cli::try_parse_from(["magi", "team", "rename", "old", "new"]).expect("parse");
    let Some(Command::Team {
        command: TeamCommand::Rename { old, new },
    }) = cli.command
    else {
        panic!("expected team rename");
    };
    assert_eq!(old, "old");
    assert_eq!(new, "new");
}

#[test]
fn parses_registration_add() {
    let cli = Cli::try_parse_from([
        "magi",
        "registration",
        "add",
        "--team",
        "core",
        "--agent",
        "alice",
        "--type",
        "codex",
        "--project",
        "/tmp/project",
        "--session",
        "s1",
    ])
    .expect("parse");
    let Some(Command::Registration {
        command:
            RegistrationCommand::Add {
                team,
                agent,
                agent_type,
                project,
                session,
            },
    }) = cli.command
    else {
        panic!("expected registration add");
    };
    assert_eq!(team, "core");
    assert_eq!(agent, "alice");
    assert_eq!(agent_type, "codex");
    assert_eq!(project.as_path(), std::path::Path::new("/tmp/project"));
    assert_eq!(session.as_deref(), Some("s1"));
}

#[test]
fn parses_registration_remove_and_reset() {
    let remove = Cli::try_parse_from([
        "magi",
        "registration",
        "remove",
        "--team",
        "core",
        "--agent",
        "alice",
    ])
    .expect("parse");
    assert!(matches!(
        remove.command,
        Some(Command::Registration {
            command: RegistrationCommand::Remove { .. }
        })
    ));

    let reset = Cli::try_parse_from([
        "magi",
        "registration",
        "reset",
        "--project",
        "/tmp/project",
        "--type",
        "codex",
        "--agent",
        "alice",
    ])
    .expect("parse");
    assert!(matches!(
        reset.command,
        Some(Command::Registration {
            command: RegistrationCommand::Reset { .. }
        })
    ));
}

#[test]
fn parses_identity_commands() {
    let list = Cli::try_parse_from([
        "magi",
        "identity",
        "list",
        "--project",
        "/tmp/project",
        "--type",
        "codex",
    ])
    .expect("parse");
    assert!(matches!(
        list.command,
        Some(Command::Identity {
            command: IdentityCommand::List { .. }
        })
    ));

    let whoami = Cli::try_parse_from([
        "magi",
        "identity",
        "whoami",
        "--project",
        "/tmp/project",
        "--type",
        "codex",
    ])
    .expect("parse");
    assert!(matches!(
        whoami.command,
        Some(Command::Identity {
            command: IdentityCommand::Whoami { .. }
        })
    ));
}

#[test]
fn parses_actas_commands() {
    let claim = Cli::try_parse_from([
        "magi",
        "actas",
        "claim",
        "alice",
        "--team",
        "core",
        "--session",
        "s1",
        "--ttl",
        "30",
    ])
    .expect("parse");
    assert!(matches!(
        claim.command,
        Some(Command::Actas {
            command: ActasCommand::Claim { .. }
        })
    ));

    let release = Cli::try_parse_from(["magi", "actas", "release", "alice", "--session", "s1"])
        .expect("parse");
    assert!(matches!(
        release.command,
        Some(Command::Actas {
            command: ActasCommand::Release { .. }
        })
    ));

    let status = Cli::try_parse_from(["magi", "actas", "status", "alice"]).expect("parse");
    assert!(matches!(
        status.command,
        Some(Command::Actas {
            command: ActasCommand::Status { .. }
        })
    ));
}

#[test]
fn parses_delivery_commands() {
    let set = Cli::try_parse_from([
        "magi",
        "delivery",
        "set",
        "both",
        "--type",
        "codex",
        "--project",
        "/tmp/project",
    ])
    .expect("parse");
    let Some(Command::Delivery {
        command:
            DeliveryCommand::Set {
                mode,
                agent_type,
                project,
            },
    }) = set.command
    else {
        panic!("expected delivery set");
    };
    assert_eq!(mode, DeliveryMode::Both);
    assert_eq!(agent_type, "codex");
    assert_eq!(project.as_path(), std::path::Path::new("/tmp/project"));

    let status = Cli::try_parse_from([
        "magi",
        "delivery",
        "status",
        "--type",
        "codex",
        "--project",
        "/tmp/project",
    ])
    .expect("parse");
    assert!(matches!(
        status.command,
        Some(Command::Delivery {
            command: DeliveryCommand::Status { .. }
        })
    ));
}
