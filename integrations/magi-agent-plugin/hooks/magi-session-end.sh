#!/usr/bin/env bash
#
# SessionEnd hook for the magi-agent plugin.
#
# Counterpart to magi-session-start.sh: when a Claude Code session ends, this
# removes the ephemeral, session-scoped magi agent that SessionStart spawned.
#
# It reads the per-session record written by SessionStart
# ($STATE_DIR/sessions/<session_id>.agent), despawns that agent, and deletes the
# record. Every step is best
# effort: a missing record, an unreachable Redis, or an already-removed agent
# never causes the hook to fail.
#
# If magi is not installed, the hook exits silently.
set -uo pipefail

# Capture the hook's JSON payload from stdin (provides session_id, reason, ...).
HOOK_INPUT="$(cat 2>/dev/null || true)"

# Resolve the magi binary: explicit override, then PATH, then install locations.
MAGI="${MAGI_BIN:-}"
[ -n "$MAGI" ] || MAGI="$(command -v magi 2>/dev/null || true)"
if [ -z "$MAGI" ]; then
  for c in "$HOME/.agents/skills/magi/bin/magi" "$HOME/.local/bin/magi"; do
    [ -x "$c" ] && MAGI="$c" && break
  done
fi
{ [ -n "$MAGI" ] && [ -x "$MAGI" ]; } || exit 0

# Extract a top-level string field from the hook's JSON payload without jq.
json_string() {
  printf '%s' "$HOOK_INPUT" |
    sed -n "s/.*\"$1\"[[:space:]]*:[[:space:]]*\"\([^\"]*\)\".*/\1/p" |
    head -1
}

STATE_DIR="${MAGI_AGENT_STATE_DIR:-${XDG_STATE_HOME:-$HOME/.local/state}/magi-agent}"
SESSIONS_DIR="$STATE_DIR/sessions"
SESSION_ID="$(json_string session_id)"

# Without a session id there is no record to act on.
[ -n "$SESSION_ID" ] || exit 0
session_file="$SESSIONS_DIR/$(printf '%s' "$SESSION_ID" | tr -cd 'A-Za-z0-9._-').agent"
[ -f "$session_file" ] || exit 0

# Two positional lines: agent name and team.
name="$(sed -n '1p' "$session_file" 2>/dev/null || true)"
team="$(sed -n '2p' "$session_file" 2>/dev/null || true)"

# Remove the ephemeral agent from its team (idempotent; tolerates absence).
if [ -n "$name" ] && [ -n "$team" ]; then
  "$MAGI" agent despawn --team "$team" --name "$name" >/dev/null 2>&1 || true
fi

# Drop the per-session record now that it has been acted on.
rm -f "$session_file" 2>/dev/null || true

exit 0
