# magi-agent (Claude Code plugin)

Event-driven bridge that turns **incoming magi messages into a live Claude
session**. The moment a teammate sends you a magi message, its text becomes a new
user turn in a persistent [Claude Agent SDK](https://code.claude.com/docs/en/agent-sdk)
conversation, and the assistant's reply is delivered back through `magi send`.

This is the "Plan A" architecture: one long-lived process holding persistent SDK
sessions (one per peer), fed by magi's Redis Pub/Sub stream (`magi watch`), so
delivery is instant rather than polled.

## Requirements

- [`magi`](https://github.com/kent8192/magi) installed and a reachable Redis
  (`magi redis status`), with `identity.active_agent` set.
- The `claude` CLI on `PATH` (the SDK drives it; uses your existing Claude auth).
- Node.js ≥ 22.18 (runs the TypeScript bridge via native type-stripping) and npm.

## Install

The repository-root `./install.sh` registers this plugin automatically (best
effort) from the current checkout's `magi` marketplace:

```bash
claude plugin marketplace add /absolute/path/to/magi
claude plugin install magi-agent@magi
# restart Claude Code
```

For development on this plugin in isolation, add the plugin directory itself as
the marketplace instead:

```bash
/plugin marketplace add /absolute/path/to/magi/integrations/magi-agent-plugin
/plugin install magi-agent@magi
# restart Claude Code
```

## Use

```bash
/magi-system setup     # one-time: npm install the Claude Agent SDK
/magi-system start     # start the daemon (auto-replies to messages addressed to you)
/magi-system status
/magi-system logs
/magi-system stop
```

Or call the controller directly: `integrations/magi-agent-plugin/bin/magi-agentd <subcommand>`.

## Layout

```
magi-agent-plugin/
├── .claude-plugin/
│   ├── plugin.json              # plugin manifest
│   └── marketplace.json         # local marketplace ("magi")
├── bin/magi-agentd              # lifecycle controller (setup/start/stop/status/logs/run)
├── lib/
│   ├── magi_agent_bridge.ts     # the bridge (TypeScript, run via Node type-stripping)
│   ├── package.json             # @anthropic-ai/claude-agent-sdk dependency
│   └── node_modules/            # installed by `setup` (gitignored)
├── commands/magi-system.md      # /magi-system slash command
├── hooks/
│   ├── hooks.json               # SessionStart + SessionEnd hook registration
│   ├── magi-session-start.sh    # startup: report state, spawn an ephemeral agent
│   └── magi-session-end.sh      # shutdown: despawn the agent, restore identity
└── skills/
    ├── magi-agent/SKILL.md      # the autonomous bridge
    └── magi-messaging/SKILL.md  # manual magi CLI usage in-session
```

## Session lifecycle hooks

On every Claude Code session start the plugin runs `hooks/magi-session-start.sh`,
which detects the magi system state (Redis reachable? identity set? bridge up?)
and injects a one-line status as session context. It never consumes your inbox,
and it never boots Redis or the bridge unless you opt in:

- `MAGI_AGENT_AUTOSTART_REDIS=1` — start managed Redis at session start if it is down.
- `MAGI_AGENT_AUTOSTART_BRIDGE=1` — start the `/magi-system` bridge daemon at session start.

### Ephemeral session agent

When Redis is reachable and `identity.active_team` is set, the SessionStart hook
also spawns a fresh, uniquely named MAGI agent for the session (`magi agent
spawn`) — e.g. `quiet-melchior` — and adopts it as the active identity. The
assigned name, team, and the previously active identity are recorded under the
daemon state dir keyed by the Claude session id. On session end,
`hooks/magi-session-end.sh` despawns that agent (`magi agent despawn`) and
restores the previous identity.

This is on by default; disable it with `MAGI_AGENT_EPHEMERAL=0`. Spawning is
idempotent per session (a re-fired SessionStart does not create duplicates), and
both hooks are best-effort — they never block session start or end.

> Single-session-per-`$HOME` assumption: the active identity is shared per
> `$HOME`, so the backup/restore is safe when one session is active per `$HOME`
> at a time (the recommended multi-agent layout uses a separate `$HOME` per
> agent). Concurrent sessions in one `$HOME` are not yet supported.

If magi is not installed, the hooks exit silently.

## Safety

Tools are **disabled by default** — the agent only converses. Loop prevention,
scope (`direct`/`team`), and a sender allowlist are built in. See
`skills/magi-agent/SKILL.md` for all configuration knobs and the security notes
before enabling tools (unattended tool use has real side effects).
