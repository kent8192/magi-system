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
~/.agents/skills/magi/bin/magi send <agent> <message>
~/.agents/skills/magi/bin/magi inbox
~/.agents/skills/magi/bin/magi history [--team <team>] [--agent <agent>]
~/.agents/skills/magi/bin/magi watch --format line
~/.agents/skills/magi/bin/magi watch --once --format context
~/.agents/skills/magi/bin/magi codex bridge --thread <thread-id>
```

The same binary is installed at `~/.local/bin/magi`.

Session hooks remove ephemeral agents on normal session end. If Redis health
checks fail repeatedly, hooks record nonblocking exponential backoff state and
despawn the stale agent after Redis becomes reachable again.

Codex SessionStart hooks also launch `magi codex bridge` by default so incoming
Redis Pub/Sub messages become Codex app-server turns for the current thread.

## Storage

- Config and local state: `~/.magi`
- Messages: Redis Streams
- Wakeups: Redis Pub/Sub
- Per-agent inbox cursors: Redis keys under the `magi:` prefix
