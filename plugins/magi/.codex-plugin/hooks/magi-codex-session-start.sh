#!/usr/bin/env bash
#
# SessionStart hook for the magi Codex plugin.
#
# When Redis is reachable and an active team is set, this hook registers a
# session-scoped Codex agent (`magi agent spawn --type codex`) and emits a
# concise SessionStart context line.
# Disable the lifecycle with MAGI_CODEX_EPHEMERAL=0.
# Disable app-server live injection with MAGI_CODEX_APP_SERVER_BRIDGE=0.
# Disable managed Codex app-server daemon autostart with
# MAGI_CODEX_APP_SERVER_DAEMON=0.
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
MAGI_CMD=("$MAGI")
if [ -n "${MAGI_BIN_SHELL:-}" ]; then
  MAGI_CMD=("$MAGI_BIN_SHELL" "$MAGI")
fi

magi_cmd() { "${MAGI_CMD[@]}" "$@"; }

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

redis_reachable() { magi_cmd redis status >/dev/null 2>&1; }

team_member_status() {
  local team_name="$1" name="$2"
  [ -n "$team_name" ] && [ -n "$name" ] || { printf 'error\n'; return 0; }
  local members
  if ! members="$(magi_cmd team members --team "$team_name" 2>/dev/null)"; then
    printf 'error\n'
    return 0
  fi
  if printf '%s\n' "$members" |
    awk -v target="$name" 'NF > 0 && $1 == target { found = 1 } END { exit found ? 0 : 1 }'; then
    printf 'found\n'
  else
    printf 'missing\n'
  fi
}

remove_current_if_matches() {
  local current_path="$1" name="$2"
  [ -n "$current_path" ] && [ -f "$current_path" ] || return 0
  [ "$(sed -n '1p' "$current_path" 2>/dev/null || true)" = "$name" ] &&
    rm -f "$current_path" 2>/dev/null || true
}

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
  magi_cmd redis start >/dev/null 2>&1 || true
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
team="$(sanitize "$(magi_cmd config get identity.active_team 2>/dev/null)")"
recorded_agent=""

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
  recorded_agent="$(sanitize "$(sed -n '1p' "$session_file" 2>/dev/null)")"
  session_team="$(sanitize "$(sed -n '2p' "$session_file" 2>/dev/null)")"
  [ -n "$session_team" ] && team="$session_team"
  if [ "$redis_state" = "reachable" ] && [ -n "$recorded_agent" ] && [ -n "$team" ]; then
    case "$(team_member_status "$team" "$recorded_agent")" in
      found)
        agent="$recorded_agent"
        ;;
      missing)
        rm -f "$session_file" "${session_file%.agent}.health" 2>/dev/null || true
        remove_current_if_matches "$current_file" "$recorded_agent"
        ;;
    esac
  fi
fi

if ephemeral_on && [ "$redis_state" = "reachable" ] && [ -n "$team" ] \
  && [ -n "$session_file" ] && [ ! -f "$session_file" ]; then
  spawned="$(sanitize "$(magi_cmd agent spawn --type codex 2>/dev/null | tail -n1)")"
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

bridge_socket_args=()
if [ -n "${MAGI_CODEX_APP_SERVER_SOCKET:-}" ]; then
  bridge_socket_args=(--socket "$MAGI_CODEX_APP_SERVER_SOCKET")
fi

daemon_auto_on() {
  case "$(printf '%s' "${MAGI_CODEX_APP_SERVER_DAEMON:-1}" | tr '[:upper:]' '[:lower:]')" in
    0 | false | no | off) return 1 ;;
    *) return 0 ;;
  esac
}

codex_daemon_running() {
  local codex_cli="${MAGI_CODEX_CLI:-codex}"
  local status
  if [ -n "${MAGI_CODEX_CLI_SHELL:-}" ]; then
    [ -x "$codex_cli" ] || return 1
    status="$("$MAGI_CODEX_CLI_SHELL" "$codex_cli" app-server daemon version 2>/dev/null)" || return 1
  else
    command -v "$codex_cli" >/dev/null 2>&1 || return 1
    status="$("$codex_cli" app-server daemon version 2>/dev/null)" || return 1
  fi
  printf '%s\n' "$status" | grep -q '"status"[[:space:]]*:[[:space:]]*"running"'
}

ensure_codex_daemon() {
  [ -z "${MAGI_CODEX_APP_SERVER_SOCKET:-}" ] || return 0
  daemon_auto_on || return 0
  codex_daemon_running && return 0
  local codex_cli="${MAGI_CODEX_CLI:-codex}"
  if [ -n "${MAGI_CODEX_CLI_SHELL:-}" ]; then
    [ -x "$codex_cli" ] || return 0
    "$MAGI_CODEX_CLI_SHELL" "$codex_cli" app-server daemon start >/dev/null 2>&1 || true
  else
    command -v "$codex_cli" >/dev/null 2>&1 || return 0
    "$codex_cli" app-server daemon start >/dev/null 2>&1 || true
  fi
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
    ensure_codex_daemon
    mkdir -p "$BRIDGES_DIR" 2>/dev/null || true
    env MAGI_SESSION_ID="$SESSION_ID" CODEX_THREAD_ID="$SESSION_ID" CODEX_SESSION_ID="$SESSION_ID" \
      MAGI_CODEX_STATE_DIR="$STATE_DIR" \
      "${MAGI_CMD[@]}" codex bridge --thread "$SESSION_ID" --cwd "$PROJECT_CWD" \
      --codex "${MAGI_CODEX_CLI:-codex}" "${bridge_socket_args[@]}" \
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
