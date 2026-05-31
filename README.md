# magi

Redis-backed cross-agent messaging for CLI AI agents.

magi is a Rust CLI that stores team membership, invites, message history, and
per-agent inbox cursors in Redis. Redis Streams are the durable message log and
Pub/Sub is used as a low-latency wakeup for `watch`.

## Install

```bash
./install.sh
```

The installer builds the Rust binary and places it at:

- `~/.agents/skills/magi/bin/magi`
- `~/.local/bin/magi`

Configuration and managed Redis state are stored under `~/.magi`.

The installer then registers the magi plugins (best effort) with whichever
agent CLIs are present — `claude` and `codex` — and `./uninstall.sh` removes
them again. See [Plugins](#plugins) below. Override the source repository or
marketplace name with the `MAGI_PLUGIN_REPO` and `MAGI_PLUGIN_MARKETPLACE`
environment variables.

## Plugins

The repository ships two plugins, both listed in the root
`.claude-plugin/marketplace.json` under the `magi-dev` marketplace:

- **`magi` (Codex)** — manifest at `.codex-plugin/plugin.json`, exposing the
  `magi` messaging skill that points agents to the Rust CLI at
  `~/.agents/skills/magi/bin/magi`.
- **`magi-agent` (Claude Code)** — the event-driven bridge under
  `integrations/magi-agent-plugin/` that turns incoming magi messages into a
  live Claude session.

`./install.sh` installs both by fetching the marketplace from GitHub:

```bash
# Claude Code
claude plugin marketplace add kent8192/magi
claude plugin install magi-agent@magi-dev

# Codex
codex plugin marketplace add kent8192/magi
codex plugin add magi@magi-dev
```

Because the marketplace is resolved from the repository's default branch, the
plugins must be published there for these commands to succeed. After installing
into Claude Code, restart it and run `/magi-system setup`.

## Quick Start

```bash
~/.local/bin/magi redis start
~/.local/bin/magi config set identity.active_agent alice
~/.local/bin/magi team create core
~/.local/bin/magi config set identity.active_team core
~/.local/bin/magi invite create --team core
```

On another agent:

```bash
~/.local/bin/magi config set redis.url <redis-url>
~/.local/bin/magi config set identity.active_agent bob
~/.local/bin/magi join --invite <token>
~/.local/bin/magi send alice "hello from bob"
```

## Commands

```bash
magi                          # interactive mode
magi redis start|status|stop
magi team create <team>
magi team list
magi team members [--team <team>]
magi invite create --team <team> [--ttl 24h]
magi invite list --team <team>
magi invite revoke <invite_id>
magi join --invite <token>
magi agent spawn [--team <team>] [--type <type>]
magi agent despawn [--team <team>] [--name <agent>]
magi send <agent> <message>
magi inbox
magi history [--team <team>] [--agent <agent>]
magi watch [--format line|json]
magi ssh start|status|stop
magi config get <key>
magi config set <key> <value>
```

## Redis

`magi redis start` prefers Docker and falls back to `redis-server` when Docker
is unavailable. Redis auth is generated and written into `~/.magi/config.toml`;
passwords are not passed on the command line.

## Legacy Scripts

The old Bash/SQLite scripts are retired. They now exit with a clear retirement
notice and point callers to the Rust CLI.
