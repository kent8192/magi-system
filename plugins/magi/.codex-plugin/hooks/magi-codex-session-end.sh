#!/usr/bin/env bash
#
# SessionEnd hook for the magi Codex plugin.
#
# Removes the session-scoped Codex agent created by SessionStart. All teardown
# is best effort.
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
safe_key() { printf '%s' "${1:-}" | tr -cd 'A-Za-z0-9._-' ; }

STATE_DIR="${MAGI_CODEX_STATE_DIR:-${XDG_STATE_HOME:-$HOME/.local/state}/magi-codex}"
SESSIONS_DIR="$STATE_DIR/sessions"
CURRENT_DIR="$STATE_DIR/current"
BRIDGES_DIR="$STATE_DIR/bridges"
SESSION_ID="$(json_string session_id)"
if [ -z "$SESSION_ID" ]; then
  SESSION_ID="${CODEX_THREAD_ID:-${CODEX_SESSION_ID:-}}"
fi
PROJECT_CWD="$(json_string cwd)"
[ -n "$PROJECT_CWD" ] || PROJECT_CWD="${PWD:-}"

[ -n "$SESSION_ID" ] || exit 0
bridge_pid_file="$BRIDGES_DIR/$(safe_key "$SESSION_ID").pid"
if [ -f "$bridge_pid_file" ]; then
  bridge_pid="$(cat "$bridge_pid_file" 2>/dev/null || true)"
  if [ -n "$bridge_pid" ]; then
    kill "$bridge_pid" 2>/dev/null || true
  fi
  rm -f "$bridge_pid_file" 2>/dev/null || true
fi

session_file="$SESSIONS_DIR/$(safe_key "$SESSION_ID").agent"
[ -f "$session_file" ] || exit 0

name="$(sed -n '1p' "$session_file" 2>/dev/null || true)"
team="$(sed -n '2p' "$session_file" 2>/dev/null || true)"

if [ -n "$name" ] && [ -n "$team" ]; then
  "$MAGI" agent despawn --team "$team" --name "$name" >/dev/null 2>&1 || true
fi

rm -f "$session_file" 2>/dev/null || true
rm -f "${session_file%.agent}.health" 2>/dev/null || true
if [ -n "$PROJECT_CWD" ] && [ -n "$name" ]; then
  current_file="$CURRENT_DIR/$(safe_key "$PROJECT_CWD").agent"
  current_name="$(sed -n '1p' "$current_file" 2>/dev/null || true)"
  [ "$current_name" = "$name" ] && rm -f "$current_file" 2>/dev/null || true
fi

exit 0
