---
name: magi
description: >-
  Send and read cross-agent messages over the magi CLI from Codex. Use when the
  user wants to message another agent, check their magi inbox, view message
  history, manage teams/invites, or watch for incoming magi messages. Triggers
  on: magi send, magi inbox, magi history, message another agent, check
  messages, magi team, magi にメッセージ, 受信確認.
---

# magi

Manual, in-session use of the [`magi`](https://github.com/kent8192/magi) CLI for
cross-agent messaging from Codex.

**Always operate through the `magi` CLI. Never read or edit `~/.magi`, the Redis
data, or installed skill files directly.**

## Preflight

```bash
magi redis status                       # backend must be reachable
magi config get identity.active_team    # your active team
```

If Redis is down: `magi redis start`. If the team is unset:
`magi config set identity.active_team <team>`. Agent names are session-scoped;
prefer the `magi-system context` injected by Codex hooks for this session's
current agent name. Use `magi agent spawn --team <team> --type codex` only for
manual lifecycle control.

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
- In a Codex session, `send`, `inbox`, `history`, and `watch` use hook-created
  session state. The hook-injected `magi-system context` is the preferred source
  for this session's agent name; the CLI has no persistent active-agent
  fallback.

## Onboarding another agent

```bash
magi invite create --team <team>        # produces a token
# on the other agent:
magi join --invite <token>
magi config set identity.active_team <team>   # join does not set the active team
```

## Notes for Codex

- The autonomous auto-reply bridge (the `/magi-system` command and `magi-agent`
  skill) is a **Claude Code** feature, built on the Claude Agent SDK; there is
  no Codex SDK auto-reply equivalent. Codex live delivery is handled by the
  Codex app-server bridge described below.
- The ephemeral session-agent lifecycle is wired to Codex SessionStart,
  UserPromptSubmit, and SessionEnd hooks. When Redis is reachable and an active
  team is set, Codex automatically runs `magi agent spawn --type codex` for the
  session and despawns it on session end. If SessionStart did not record the
  current session, UserPromptSubmit self-heals by spawning and recording before
  injecting context. On each prompt, Codex receives the current magi-system
  context: session id, active agent, active team, Redis state, and session
  record status. Treat that hook-derived active agent as authoritative for
  self-identification. Three due consecutive Redis health-check failures mark
  the recorded agent for cleanup with a 1s, 2s, then 4s nonblocking backoff; the
  next reachable hook run despawns it and clears the stale record. Disable
  spawning with `MAGI_CODEX_EPHEMERAL=0`.
- SessionStart also launches `magi codex bridge --thread <session-id>` unless
  `MAGI_CODEX_APP_SERVER_BRIDGE=0`. The bridge subscribes to Redis Pub/Sub for
  this session agent, consumes unread inbox messages, and sends each
  `<sender>-><recipient>: message` line to the Codex app-server with
  `turn/start`. If Codex rejects the new turn because another turn is active,
  the bridge falls back to `thread/inject_items` so the message is still present
  in model-visible thread history. Prompt hooks report the bridge as starting,
  running, retrying, stopped, or disabled. Set `MAGI_CODEX_CLI` when the desired
  Codex CLI is not the first `codex` on PATH.
