---
name: magi
description: Redis-backed cross-agent messaging. Send messages between CLI agents with managed Redis, Streams, Pub/Sub wakeups, and an interactive watch mode.
---

# magi

Use the Rust CLI. Do not read or edit `~/.magi` files directly.

## Commands

```bash
~/.agents/skills/magi/bin/magi              # interactive mode
~/.agents/skills/magi/bin/magi redis start  # Docker first, redis-server fallback
~/.agents/skills/magi/bin/magi redis reset  # clear managed Redis data and restart
~/.agents/skills/magi/bin/magi team create <team>
~/.agents/skills/magi/bin/magi invite create --team <team>
~/.agents/skills/magi/bin/magi join --invite <token>
~/.agents/skills/magi/bin/magi agent spawn   # register a unique <adjective>-<magi> agent
~/.agents/skills/magi/bin/magi agent despawn # remove it again
~/.agents/skills/magi/bin/magi registration add --team <team> --agent <agent> --type <type> --project <path>
~/.agents/skills/magi/bin/magi identity whoami --project <path> --type <type>
~/.agents/skills/magi/bin/magi actas claim <agent> [--team <team>] [--session <id>]
~/.agents/skills/magi/bin/magi delivery set both --type <type> --project <path>
~/.agents/skills/magi/bin/magi send <agent> <message>
~/.agents/skills/magi/bin/magi inbox [--team <team>] [--agent <agent>] [--quiet]
~/.agents/skills/magi/bin/magi history [--team <team>] [--agent <agent>] [--limit <n>]
~/.agents/skills/magi/bin/magi watch --format line
~/.agents/skills/magi/bin/magi watch --once --format context
~/.agents/skills/magi/bin/magi codex bridge --thread <thread-id> [--socket <sock>]
```

The same binary is installed at `~/.local/bin/magi`.

Session hooks remove ephemeral agents on normal session end. If Redis health
checks fail repeatedly, hooks record nonblocking exponential backoff state and
despawn the stale agent after Redis becomes reachable again.

Use `registration` and `identity` commands for explicit project/type operator
discovery. Use `actas` when a session must exclusively claim a role before
consuming its inbox. Delivery mode is stored by `delivery` commands for a
project/type pair; Codex hooks create the default explicit `both` mode for the
current `(codex, cwd)` pair when Redis is reachable.

Codex hooks keep the managed Codex app-server daemon reachable on SessionStart,
UserPromptSubmit, PreToolUse, PostToolUse, Stop, and SessionEnd unless
`MAGI_CODEX_APP_SERVER_DAEMON=0` opts out. The hook checks
`codex app-server daemon version`, starts the daemon when it is not reachable,
and retries once with `codex app-server daemon restart` if start does not repair
the control socket. SessionStart then launches `magi codex bridge` by default,
so incoming Redis Pub/Sub messages are injected into the current Codex thread
before the bridge best-effort starts a Codex app-server turn. Prompt hooks run
the same daemon check even when a bridge process already exists, then restart a
missing bridge and only report a Codex `agent` when Redis confirms the recorded
name is still in the active team roster. Stale records are cleared and
self-healed through the normal spawn path; unreachable Redis produces `agent:
unset` instead of an unverified reply target. Prompt hooks report whether the
bridge is starting, running, delivering, injecting, injected, turn_started,
retrying, unsupported, stopped, or disabled. The bridge connects to the Codex
app-server over the Unix control socket's WebSocket transport. A `retrying`
bridge keeps the last app-server injection error visible until a later message
is successfully injected; empty inbox checks do not clear that error. An
`unsupported` bridge means the Codex runtime does not expose a reachable
app-server control socket. Set `MAGI_CODEX_APP_SERVER_SOCKET` to a Unix socket
path when Codex is running with one; `stdio://` app-server processes cannot be
reached by the external bridge. Set `MAGI_CODEX_APP_SERVER_DAEMON=0` to disable
managed daemon autostart.

## Storage

- Config and local state: `~/.magi`
- Messages: Redis Streams
- Wakeups: Redis Pub/Sub
- Per-agent inbox cursors: Redis keys under the `magi:` prefix
