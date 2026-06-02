---
description: Redis-backed agent messaging — inbox, send, history, team, watch
---

Use `~/.agents/skills/__SKILL_NAME__/bin/magi` for all messaging operations.
Do not read or edit `~/.magi` directly.

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
~/.agents/skills/__SKILL_NAME__/bin/magi redis reset
~/.agents/skills/__SKILL_NAME__/bin/magi agent spawn
~/.agents/skills/__SKILL_NAME__/bin/magi agent despawn
~/.agents/skills/__SKILL_NAME__/bin/magi watch --format line
~/.agents/skills/__SKILL_NAME__/bin/magi watch --once --format context
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
