---
name: setup-magi
description: >-
  Set up MAGI SYSTEM from Codex. Use when the user asks to set up MAGI SYSTEM,
  Setup MAGI SYSTEM, Set up MAGI SYSTEM, or MAGI SYSTEM セットアップ.
---

# setup-magi

Use this skill to set up MAGI SYSTEM through the repository-provided setup
entrypoint. Do not reimplement the setup steps in the prompt hook or by editing
configuration files directly.

## Procedure

1. Read the nearest `setup.sh` before running it. From this `SKILL.md`, the
   expected relative path is `../../setup.sh`.
2. If the user provided a team name, run:
   ```bash
   MAGI_SETUP_TEAM=<team> ../../setup.sh
   ```
3. If the user did not provide a team name, run:
   ```bash
   ../../setup.sh
   ```
4. Report the setup result and any next command printed by `setup.sh`.

`setup.sh` must remain the source of truth for the exact setup behavior. It uses
only the `magi` CLI to start managed Redis, create the setup team when needed,
and set `identity.active_team`.
