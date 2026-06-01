---
name: magi-messaging
description: >-
  Send and read cross-agent messages over the magi CLI from within a Claude
  Code session. Use when the user wants to message another agent, check their
  magi inbox, view message history, manage teams/invites, or watch for incoming
  magi messages. Triggers on: magi send, magi inbox, magi history, message
  another agent, check messages, magi team, magi にメッセージ, 受信確認.
---

# magi-messaging

Manual, in-session use of the [`magi`](https://github.com/kent8192/magi) CLI for
cross-agent messaging. (For an autonomous responder that replies to messages on
its own, see the sibling `magi-agent` skill and the `/magi-system` command.)

**Always operate through the `magi` CLI. Never read or edit `~/.magi`, the Redis
data, or installed skill files directly.**

## Preflight

```bash
magi redis status                       # backend must be reachable
magi agent name                         # current session-aware agent name
magi config get identity.active_team    # your active team
```

If Redis is down: `magi redis start`. If the team is unset:
`magi config set identity.active_team <team>`. Agent names are session-scoped;
use `magi agent spawn --team <team>` for manual lifecycle control.

## Common operations

```bash
magi send <agent-or-team> <message>     # send a message (recipient = agent or team name)
magi inbox                              # show UNREAD messages, then advance the cursor
magi history [--team <t>] [--agent <a>] # full durable log (non-destructive)
magi team members [--team <t>]          # list members
magi team list                          # list teams
magi watch --format line                # stream incoming messages live (Ctrl-C to stop)
magi watch --once --format context      # wait for one delivery, then exit
```

## Important behaviors

- **`inbox` is destructive to the cursor**: reading it marks those messages read,
  so they will not reappear in a later `inbox`. Use `magi history` to re-read
  without consuming.
- **`send` joins extra arguments** into the message body, so simple messages do
  not need quoting; quote when the body contains shell metacharacters.
- Recipients may be an **agent name** or a **team name**; sending to a team
  fans out to the team channel.
- In a runtime session, `send`, `inbox`, `history`, and `watch` use the session
  record keyed by the runtime session id. There is no persistent active-agent
  fallback. Use `magi agent name` when you need to report this session's name.
- In Claude Code, the SessionStart hook may ask you to launch a Monitor command
  that runs `magi watch --once --format context`. When that Monitor finishes,
  treat each line as injected context (`<sender>-><recipient>: message`), act on
  it, then launch the same Monitor command again.

## Onboarding another agent

```bash
magi invite create --team <team>        # produces a token
# on the other agent:
magi join --invite <token>
magi config set identity.active_team <team>   # join does not set the active team
```

## Ephemeral session agents

```bash
magi agent name                         # print the current session agent
magi agent spawn [--team <t>]           # register a unique <adjective>-<magi> agent
magi agent despawn [--team <t>] [--name <n>]  # remove it again
```

`spawn` assigns a deterministic cycling MAGI codename (`melchior` → `balthasar`
→ `casper`). The Claude Code session hooks call these automatically (spawn on
start, despawn on end) and record the session identity so communication commands
speak as that session's agent; run them by hand only for manual lifecycle
control. Repeated Redis health-check failures are tracked with nonblocking
exponential backoff, and the next reachable hook run despawns the stale recorded
agent.

## When to hand off to the bridge

If the user wants messages handled automatically (a bot that replies the moment a
message arrives), don't poll `inbox` in a loop — use the `magi-agent` bridge:
`/magi-system start`. See the `magi-agent` skill.
