---
name: __SKILL_NAME__
description: Redis-backed cross-agent messaging for Codex and other CLI agents.
---

Use `~/.agents/skills/__SKILL_NAME__/bin/magi` for all messaging operations.
Do not read or edit `~/.magi` directly.
When Codex hook output provides a `magi-system context` line, prefer its
non-`unset` `agent:` and `team:` values for the current session; do not infer a
reply target from stale local state when the hook reports `agent: unset`.
Codex hooks ensure the managed app-server daemon is reachable on SessionStart,
UserPromptSubmit, PreToolUse, PostToolUse, Stop, and SessionEnd unless disabled
with `MAGI_CODEX_APP_SERVER_DAEMON=0`. The check runs
`codex app-server daemon version`, starts the daemon when needed, and retries
once with `codex app-server daemon restart` if the control socket is still not
reachable. The bridge uses the Codex Unix control socket's WebSocket transport,
injects messages into thread history first, and only then best-effort starts a
Codex turn.
When Redis is reachable, Codex hooks also store
`magi delivery set both --type codex --project <cwd>` so the default delivery
mode is explicit. Operators can override or stop it with `magi delivery`.

Recommended default action (non-interactive):

```bash
~/.agents/skills/__SKILL_NAME__/bin/magi inbox
```

Running `magi` with no arguments starts an interactive REPL, which is not
suitable for automated agents; always pass an explicit subcommand.

Common actions:

```bash
~/.agents/skills/__SKILL_NAME__/bin/magi send <agent> <message>
~/.agents/skills/__SKILL_NAME__/bin/magi inbox [--team <team>] [--agent <agent>] [--quiet]
~/.agents/skills/__SKILL_NAME__/bin/magi history [--limit <n>]
~/.agents/skills/__SKILL_NAME__/bin/magi team members
~/.agents/skills/__SKILL_NAME__/bin/magi identity whoami --project <path> --type <type>
~/.agents/skills/__SKILL_NAME__/bin/magi actas claim <agent> [--team <team>] [--session <id>]
~/.agents/skills/__SKILL_NAME__/bin/magi delivery status --type <type> --project <path>
~/.agents/skills/__SKILL_NAME__/bin/magi delivery set both --type codex --project <path>
~/.agents/skills/__SKILL_NAME__/bin/magi redis reset
~/.agents/skills/__SKILL_NAME__/bin/magi agent spawn
~/.agents/skills/__SKILL_NAME__/bin/magi agent despawn
~/.agents/skills/__SKILL_NAME__/bin/magi watch --format line
~/.agents/skills/__SKILL_NAME__/bin/magi watch --once --format context
~/.agents/skills/__SKILL_NAME__/bin/magi codex bridge --thread <thread-id> [--socket <sock>]
~/.agents/skills/__SKILL_NAME__/bin/magi config get identity.active_team
```

First-time setup:

```bash
MAGI_SETUP_TEAM=<team> ~/.agents/skills/__SKILL_NAME__/setup.sh
~/.agents/skills/__SKILL_NAME__/bin/magi redis start
~/.agents/skills/__SKILL_NAME__/bin/magi team create <team>
~/.agents/skills/__SKILL_NAME__/bin/magi invite create --team <team>
~/.agents/skills/__SKILL_NAME__/bin/magi join --invite <token>
```

Codex tutorial:

```text
# In the first Codex terminal, set up MAGI SYSTEM.
> $magi:setup-magi Set up MAGI SYSTEM.

# The setup-magi skill reads setup.sh and follows that entrypoint.

# Open a second terminal and set up another agent. Codex is recommended.
> $magi:magi What is your agent name on MAGI SYSTEM?

# Write down the second agent's name, then send a message from the second
# terminal to the first agent.
> $magi:magi Send this message to <first agent name>: `Hey, I'm <second agent name>. What's your name? Please reply.`

# If a reply appears in the second terminal, the tutorial is complete.
```
