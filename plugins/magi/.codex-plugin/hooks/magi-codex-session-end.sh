#!/usr/bin/env bash
#
# SessionEnd hook for the magi Codex plugin.
#
# Removes the session-scoped Codex agent created by SessionStart and restores
# the previously active identity. All teardown is best effort.
set -uo pipefail

HOOK_INPUT="$(cat 2>/dev/null || true)"

MAGI="${MAGI_BIN:-}"
[ -n "$MAGI" ] || MAGI="$(command -v magi 2>/dev/null || true)"
if [ -z "$MAGI" ]; then
  for c in "$HOME/.agents/skills/magi/bin/magi" "$HOME/.local/bin/magi"; do
    [ -x "$c" ] && MAGI="$c" && break
  done
fi
{ [ -n "$MAGI" ] && [ -x "$MAGI" ]; } || exit 0

json_string() {
  printf '%s' "$HOOK_INPUT" |
    sed -n "s/.*\"$1\"[[:space:]]*:[[:space:]]*\"\([^\"]*\)\".*/\1/p" |
    head -1
}

STATE_DIR="${MAGI_CODEX_STATE_DIR:-${XDG_STATE_HOME:-$HOME/.local/state}/magi-codex}"
SESSIONS_DIR="$STATE_DIR/sessions"
SESSION_ID="$(json_string session_id)"
if [ -z "$SESSION_ID" ]; then
  SESSION_ID="${CODEX_THREAD_ID:-${CODEX_SESSION_ID:-}}"
fi

[ -n "$SESSION_ID" ] || exit 0
session_file="$SESSIONS_DIR/$(printf '%s' "$SESSION_ID" | tr -cd 'A-Za-z0-9._-').agent"
[ -f "$session_file" ] || exit 0

name="$(sed -n '1p' "$session_file" 2>/dev/null || true)"
team="$(sed -n '2p' "$session_file" 2>/dev/null || true)"
prev_agent="$(sed -n '3p' "$session_file" 2>/dev/null || true)"

if [ -n "$name" ] && [ -n "$team" ]; then
  "$MAGI" agent despawn --team "$team" --name "$name" >/dev/null 2>&1 || true
fi

"$MAGI" config set identity.active_agent "$prev_agent" >/dev/null 2>&1 || true
rm -f "$session_file" 2>/dev/null || true

exit 0
