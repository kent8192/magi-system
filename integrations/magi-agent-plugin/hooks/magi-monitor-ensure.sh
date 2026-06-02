#!/usr/bin/env bash
#
# Ensure a Claude Code Monitor task is waiting for this session's magi inbox.
#
# Hook phases call this script after SessionStart. It only emits a directive for
# Claude Code to launch the Monitor tool; it never consumes the inbox itself.
set -uo pipefail

HOOK_INPUT="$(cat 2>/dev/null || true)"
PLUGIN_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

MAGI="${MAGI_BIN:-}"
[ -n "$MAGI" ] || MAGI="$(command -v magi 2>/dev/null || true)"
if [ -z "$MAGI" ]; then
  for c in "$HOME/.agents/skills/magi/bin/magi" "$HOME/.local/bin/magi"; do
    [ -x "$c" ] && MAGI="$c" && break
  done
fi
{ [ -n "$MAGI" ] && [ -x "$MAGI" ]; } || exit 0

sanitize() { printf '%s' "${1:-}" | tr -d '"\\\n\r' ; }
safe_key() { printf '%s' "${1:-}" | tr -cd 'A-Za-z0-9._-' ; }

json_string() {
  printf '%s' "$HOOK_INPUT" |
    sed -n "s/.*\"$1\"[[:space:]]*:[[:space:]]*\"\([^\"]*\)\".*/\1/p" |
    head -1
}

json_escape() {
  printf '%s' "${1:-}" |
    sed 's/\\/\\\\/g; s/"/\\"/g; s/	/\\t/g'
}

monitor_enabled() {
  case "$(printf '%s' "${MAGI_AGENT_MONITOR:-1}" | tr '[:upper:]' '[:lower:]')" in
    0 | false | no | off) return 1 ;;
    *) return 0 ;;
  esac
}

redis_reachable() { "$MAGI" redis status >/dev/null 2>&1; }

monitor_pid_file() {
  printf '%s/monitors/%s.pid' "$STATE_DIR" "$(safe_key "$1")"
}

monitor_running() {
  local file="$1" pid
  [ -f "$file" ] || return 1
  pid="$(sed -n '1p' "$file" 2>/dev/null || true)"
  if [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null; then
    return 0
  fi
  rm -f "$file" 2>/dev/null || true
  return 1
}

emit_system_message() {
  local message
  message="$(json_escape "$1")"
  printf '{"continue":true,"systemMessage":"%s"}\n' "$message"
}

STATE_DIR="${MAGI_AGENT_STATE_DIR:-${XDG_STATE_HOME:-$HOME/.local/state}/magi-agent}"
SESSIONS_DIR="$STATE_DIR/sessions"
SESSION_ID="$(json_string session_id)"

[ -n "$SESSION_ID" ] || exit 0
monitor_enabled || exit 0
redis_reachable || exit 0

session_file="$SESSIONS_DIR/$(safe_key "$SESSION_ID").agent"
[ -f "$session_file" ] || exit 0

agent="$(sanitize "$(sed -n '1p' "$session_file" 2>/dev/null)")"
team="$(sanitize "$(sed -n '2p' "$session_file" 2>/dev/null)")"
[ -n "$agent" ] && [ -n "$team" ] || exit 0

pid_file="$(monitor_pid_file "$SESSION_ID")"
if monitor_running "$pid_file"; then
  emit_system_message "magi Monitor already running for this Claude Code session."
  exit 0
fi

monitor_script="$PLUGIN_ROOT/hooks/magi-monitor-once.sh"
monitor_session="$(sanitize "$SESSION_ID")"
monitor_command="$(sanitize "$monitor_script $monitor_session")"
emit_system_message "Claude Code Monitor directive: before other work, invoke Monitor with command: $monitor_command ; description: magi inbox once ; persistent: false. When the Monitor finishes, treat each output line as injected magi context in the form <sender>-><recipient>: message, act on it, then invoke the same Monitor command again so this session keeps waiting for the next Redis Pub/Sub message."
exit 0
