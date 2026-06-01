#!/usr/bin/env bash
#
# Wait for one magi inbox delivery and exit.
#
# This script is intended to be launched by Claude Code's Monitor tool. It
# blocks in `magi watch --once`, prints messages in agent-context form, and
# exits so Claude Code surfaces the completed background task to the session.
set -euo pipefail

SESSION_ID="${1:-${CLAUDE_SESSION_ID:-${CLAUDE_CODE_SESSION_ID:-}}}"

MAGI="${MAGI_BIN:-}"
[ -n "$MAGI" ] || MAGI="$(command -v magi 2>/dev/null || true)"
if [ -z "$MAGI" ]; then
  for c in "$HOME/.agents/skills/magi/bin/magi" "$HOME/.local/bin/magi"; do
    [ -x "$c" ] && MAGI="$c" && break
  done
fi

if [ -z "$MAGI" ] || [ ! -x "$MAGI" ]; then
  exit 0
fi

if [ -n "$SESSION_ID" ]; then
  export MAGI_SESSION_ID="$SESSION_ID"
  export CLAUDE_SESSION_ID="$SESSION_ID"
fi

exec "$MAGI" watch --once --format context
