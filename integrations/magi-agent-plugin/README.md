# magi (Claude Code plugin)

Event-driven bridge that turns **incoming magi messages into a live Claude
session**. The moment a teammate sends you a magi message, its text becomes a new
user turn in a persistent [Claude Agent SDK](https://code.claude.com/docs/en/agent-sdk)
conversation, and the assistant's reply is delivered back through `magi send`.

This is the "Plan A" architecture: one long-lived process holding persistent SDK
sessions (one per peer), fed by magi's Redis Pub/Sub stream (`magi watch`), so
delivery is instant rather than polled.

## Requirements

- [`magi`](https://github.com/kent8192/magi) installed and a reachable Redis
  (`magi redis status`), with `MAGI_AGENT_SELF` set for the bridge daemon.
- The `claude` CLI on `PATH` (the SDK drives it; uses your existing Claude auth).
- Node.js ≥ 22.18 (runs the TypeScript bridge via native type-stripping) and npm.

## Install

The repository-root `./install.sh` registers this plugin automatically (best
effort) from the current checkout's `magi` marketplace:

```bash
claude plugin marketplace add /absolute/path/to/magi
claude plugin install magi@magi
# restart Claude Code
```

For development on this plugin in isolation, add the plugin directory itself as
the marketplace instead:

```bash
/plugin marketplace add /absolute/path/to/magi/integrations/magi-agent-plugin
/plugin install magi@magi
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
│   ├── hooks.json               # lifecycle + Monitor ensure hook registration
│   ├── magi-monitor-ensure.sh   # asks Claude Code to start Monitor if absent
│   ├── magi-monitor-once.sh     # waits for one inbox delivery
│   ├── magi-session-start.sh    # startup: report state, spawn an ephemeral agent
│   └── magi-session-end.sh      # shutdown: despawn the agent
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
- `MAGI_AGENT_MONITOR=0` — disable the Claude Code Monitor directive.

### Live Monitor delivery

When Redis is reachable and a session agent exists, SessionStart,
UserPromptSubmit, PostToolUse, and Stop hooks ensure that at least one Monitor
job is waiting for the session inbox. If no live Monitor pid is recorded, the
hook asks Claude Code to launch:

```bash
hooks/magi-monitor-once.sh <claude-session-id>
```

The wrapper runs `magi watch --once --format context`. It waits in the
background until Redis Pub/Sub announces a message for this session's MAGI
agent, prints each delivered line as `<sender>-><recipient>: message`, and
exits. Claude Code should treat the completed Monitor output as injected magi
context, act on it, and launch the same Monitor command again for the next
message. The wrapper records a session-scoped pid while it is running so later
hooks do not ask for duplicate Monitor jobs; stale pid files are ignored and
removed.

On Stop, the hook also reads Claude Code's `background_tasks` input. If no task
with `type: "monitor"` is present, it returns a blocking decision with the
Monitor launch directive so the foreground session keeps at least one Monitor
alive before stopping.

### Ephemeral session agent

When Redis is reachable and `identity.active_team` is set, the SessionStart hook
also spawns a fresh, uniquely named MAGI agent for the session (`magi agent
spawn`) — e.g. `quiet-melchior`. The assigned name and team are recorded under
the daemon state dir keyed by the Claude session id. On session end,
`hooks/magi-session-end.sh` despawns that agent (`magi agent despawn`).

This is on by default; disable it with `MAGI_AGENT_EPHEMERAL=0`. Spawning is
idempotent per session (a re-fired SessionStart does not create duplicates), and
both hooks are best-effort — they never block session start or end.

The start hook also tracks Redis health for recorded ephemeral agents without
sleeping in the hook. Three due consecutive failures use a 1s, 2s, then 4s
backoff and mark the session for cleanup; once Redis is reachable again, the hook
despawns the stale agent and clears the session record.

Messaging commands use the session record for their agent identity, so
concurrent sessions in one `$HOME` can send, read, and watch as their own
spawned MAGI agent names. There is no persistent active-agent fallback.

If magi is not installed, the hooks exit silently.

## Safety

Tools are **disabled by default** — the agent only converses. Loop prevention,
scope (`direct`/`team`), and a sender allowlist are built in. See
`skills/magi-agent/SKILL.md` for all configuration knobs and the security notes
before enabling tools (unattended tool use has real side effects).
