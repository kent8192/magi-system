#!/usr/bin/env bash
#
# SessionStart hook for the magi Codex plugin.
#
# When Redis is reachable and an active team is set, this hook registers a
# session-scoped Codex agent (`magi agent spawn --type codex`) and emits a
# concise SessionStart context line.
# Disable the lifecycle with MAGI_CODEX_EPHEMERAL=0.
# Disable app-server live injection with MAGI_CODEX_APP_SERVER_BRIDGE=0.
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
safe_key() { printf '%s' "${1:-}" | tr -cd 'A-Za-z0-9._-' ; }

json_string() {
  printf '%s' "$HOOK_INPUT" |
    sed -n "s/.*\"$1\"[[:space:]]*:[[:space:]]*\"\([^\"]*\)\".*/\1/p" |
    head -1
}

redis_reachable() { "$MAGI" redis status >/dev/null 2>&1; }

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

agent=""
team="$(sanitize "$("$MAGI" config get identity.active_team 2>/dev/null)")"

ephemeral_on() {
  case "$(printf '%s' "${MAGI_CODEX_EPHEMERAL:-1}" | tr '[:upper:]' '[:lower:]')" in
    0 | false | no | off) return 1 ;;
    *) return 0 ;;
  esac
}

session_file=""
if [ -n "$SESSION_ID" ]; then
  session_file="$SESSIONS_DIR/$(safe_key "$SESSION_ID").agent"
fi
current_file=""
if [ -n "$PROJECT_CWD" ]; then
  current_file="$CURRENT_DIR/$(safe_key "$PROJECT_CWD").agent"
fi

if [ -n "$session_file" ] && [ -f "$session_file" ]; then
  agent="$(sanitize "$(sed -n '1p' "$session_file" 2>/dev/null)")"
  session_team="$(sanitize "$(sed -n '2p' "$session_file" 2>/dev/null)")"
  [ -n "$session_team" ] && team="$session_team"
fi

if ephemeral_on && [ "$redis_state" = "reachable" ] && [ -n "$team" ] \
  && [ -n "$session_file" ] && [ ! -f "$session_file" ]; then
  spawned="$(sanitize "$("$MAGI" agent spawn --type codex 2>/dev/null | tail -n1)")"
  if [ -n "$spawned" ]; then
    mkdir -p "$SESSIONS_DIR" 2>/dev/null || true
    printf '%s\n%s\n' "$spawned" "$team" >"$session_file" 2>/dev/null || true
    agent="$spawned"
  fi
fi
if [ -n "$agent" ] && [ -n "$team" ] && [ -n "$current_file" ]; then
  mkdir -p "$CURRENT_DIR" 2>/dev/null || true
  printf '%s\n%s\n' "$agent" "$team" >"$current_file" 2>/dev/null || true
fi
if [ -n "$session_file" ] && [ "$redis_state" = "reachable" ]; then
  rm -f "${session_file%.agent}.health" 2>/dev/null || true
fi

bridge_on() {
  case "$(printf '%s' "${MAGI_CODEX_APP_SERVER_BRIDGE:-1}" | tr '[:upper:]' '[:lower:]')" in
    0 | false | no | off) return 1 ;;
    *) return 0 ;;
  esac
}

bridge_running() {
  [ -n "${1:-}" ] && [ -f "$1" ] || return 1
  local pid
  pid="$(cat "$1" 2>/dev/null || true)"
  [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null
}

status_field() {
  local file="$1" key="$2"
  [ -f "$file" ] || return 0
  sed -n "s/^${key}=//p" "$file" 2>/dev/null | head -1
}

bridge_state="stopped"
if bridge_on && [ "$redis_state" = "reachable" ] && [ -n "$agent" ] && [ -n "$team" ] \
  && [ -n "$SESSION_ID" ]; then
  bridge_pid_file="$BRIDGES_DIR/$(safe_key "$SESSION_ID").pid"
  bridge_log_file="$BRIDGES_DIR/$(safe_key "$SESSION_ID").log"
  bridge_status_file="$BRIDGES_DIR/$(safe_key "$SESSION_ID").status"
  if bridge_running "$bridge_pid_file"; then
    bridge_state="$(sanitize "$(status_field "$bridge_status_file" state)")"
    [ -n "$bridge_state" ] || bridge_state="running"
  else
    rm -f "$bridge_pid_file" 2>/dev/null || true
    mkdir -p "$BRIDGES_DIR" 2>/dev/null || true
    MAGI_SESSION_ID="$SESSION_ID" CODEX_THREAD_ID="$SESSION_ID" CODEX_SESSION_ID="$SESSION_ID" \
      MAGI_CODEX_STATE_DIR="$STATE_DIR" \
      "$MAGI" codex bridge --thread "$SESSION_ID" --cwd "$PROJECT_CWD" \
      --codex "${MAGI_CODEX_CLI:-codex}" \
      >>"$bridge_log_file" 2>&1 &
    bridge_pid="$!"
    printf '%s\n' "$bridge_pid" >"$bridge_pid_file" 2>/dev/null || true
    {
      printf 'state=starting\n'
      printf 'pid=%s\n' "$bridge_pid"
      printf 'updated_at=%s\n' "$(date +%s)"
      printf 'last_error=\n'
    } >"$bridge_status_file" 2>/dev/null || true
    bridge_state="starting"
  fi
elif ! bridge_on; then
  bridge_state="disabled"
fi

ctx="magi messaging available. Redis: ${redis_state}; agent: ${agent:-unset}; team: ${team:-unset}; codex app-server bridge: ${bridge_state}. Use the magi CLI for messaging (send/inbox/history/team); do not read or edit ~/.magi directly."
printf '{"hookSpecificOutput":{"hookEventName":"SessionStart","additionalContext":"%s"}}\n' "$ctx"
exit 0
