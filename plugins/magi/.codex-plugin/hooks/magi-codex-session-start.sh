#!/usr/bin/env bash
#
# SessionStart hook for the magi Codex plugin.
#
# When Redis is reachable and an active team is set, this hook registers a
# session-scoped Codex agent (`magi agent spawn --type codex`), records the
# previous active identity, and emits a concise SessionStart context line.
# Disable the lifecycle with MAGI_CODEX_EPHEMERAL=0.
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

truthy() {
  case "$(printf '%s' "${1:-}" | tr '[:upper:]' '[:lower:]')" in
    1 | true | yes | on) return 0 ;;
    *) return 1 ;;
  esac
}

sanitize() { printf '%s' "${1:-}" | tr -d '"\\\n\r' ; }

json_string() {
  printf '%s' "$HOOK_INPUT" |
    sed -n "s/.*\"$1\"[[:space:]]*:[[:space:]]*\"\([^\"]*\)\".*/\1/p" |
    head -1
}

redis_reachable() { "$MAGI" redis status >/dev/null 2>&1; }

STATE_DIR="${MAGI_CODEX_STATE_DIR:-${XDG_STATE_HOME:-$HOME/.local/state}/magi-codex}"
SESSIONS_DIR="$STATE_DIR/sessions"
SESSION_ID="$(json_string session_id)"
if [ -z "$SESSION_ID" ]; then
  SESSION_ID="${CODEX_THREAD_ID:-${CODEX_SESSION_ID:-}}"
fi

if ! redis_reachable && truthy "${MAGI_CODEX_AUTOSTART_REDIS:-}"; then
  "$MAGI" redis start >/dev/null 2>&1 || true
  for _ in 1 2 3 4 5 6 7 8 9 10; do
    redis_reachable && break
    sleep 0.5
  done
fi

if redis_reachable; then
  redis_state="reachable"
else
  redis_state="DOWN"
fi

agent="$(sanitize "$("$MAGI" config get identity.active_agent 2>/dev/null)")"
team="$(sanitize "$("$MAGI" config get identity.active_team 2>/dev/null)")"

ephemeral_on() {
  case "$(printf '%s' "${MAGI_CODEX_EPHEMERAL:-1}" | tr '[:upper:]' '[:lower:]')" in
    0 | false | no | off) return 1 ;;
    *) return 0 ;;
  esac
}

session_file=""
if [ -n "$SESSION_ID" ]; then
  session_file="$SESSIONS_DIR/$(printf '%s' "$SESSION_ID" | tr -cd 'A-Za-z0-9._-').agent"
fi

if ephemeral_on && [ "$redis_state" = "reachable" ] && [ -n "$team" ] \
  && [ -n "$session_file" ] && [ ! -f "$session_file" ]; then
  prev_agent="$agent"
  spawned="$(sanitize "$("$MAGI" agent spawn --type codex 2>/dev/null | tail -n1)")"
  if [ -n "$spawned" ]; then
    mkdir -p "$SESSIONS_DIR" 2>/dev/null || true
    printf '%s\n%s\n%s\n' "$spawned" "$team" "$prev_agent" >"$session_file" 2>/dev/null || true
    agent="$spawned"
  fi
fi

ctx="magi messaging available. Redis: ${redis_state}; agent: ${agent:-unset}; team: ${team:-unset}. Use the magi CLI for messaging (send/inbox/history/team); do not read or edit ~/.magi directly."
printf '{"hookSpecificOutput":{"hookEventName":"SessionStart","additionalContext":"%s"}}\n' "$ctx"
exit 0
