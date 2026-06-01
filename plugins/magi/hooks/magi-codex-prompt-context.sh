#!/usr/bin/env bash
#
# UserPromptSubmit hook for the magi Codex plugin.
#
# Emits a small magi-system status block before each prompt is processed so the
# agent can see its current session id, active magi identity, team, Redis state,
# and whether a SessionStart record exists.
set -uo pipefail

HOOK_INPUT="$(cat 2>/dev/null || true)"

MAGI="${MAGI_BIN:-}"
[ -n "$MAGI" ] || MAGI="$(command -v magi 2>/dev/null || true)"
if [ -z "$MAGI" ]; then
  for c in "$HOME/.agents/skills/magi/bin/magi" "$HOME/.local/bin/magi"; do
    [ -x "$c" ] && MAGI="$c" && break
  done
fi

sanitize() { printf '%s' "${1:-}" | tr -d '"\\\n\r' ; }
safe_key() { printf '%s' "${1:-}" | tr -cd 'A-Za-z0-9._-' ; }

ephemeral_on() {
  case "$(printf '%s' "${MAGI_CODEX_EPHEMERAL:-1}" | tr '[:upper:]' '[:lower:]')" in
    0 | false | no | off) return 1 ;;
    *) return 0 ;;
  esac
}

json_string() {
  printf '%s' "$HOOK_INPUT" |
    sed -n "s/.*\"$1\"[[:space:]]*:[[:space:]]*\"\([^\"]*\)\".*/\1/p" |
    head -1
}

now_seconds() {
  if [ -n "${MAGI_CODEX_HEALTH_NOW:-}" ]; then
    printf '%s\n' "$MAGI_CODEX_HEALTH_NOW"
  else
    date +%s
  fi
}

health_field() {
  local file="$1" key="$2"
  [ -f "$file" ] || return 0
  sed -n "s/^${key}=//p" "$file" 2>/dev/null | head -1
}

write_health_state() {
  local file="$1" name="$2" team_name="$3" failures="$4" next_check_at="$5" cleanup_pending="$6"
  mkdir -p "$(dirname "$file")" 2>/dev/null || true
  {
    printf 'agent=%s\n' "$name"
    printf 'team=%s\n' "$team_name"
    printf 'failures=%s\n' "$failures"
    printf 'next_check_at=%s\n' "$next_check_at"
    printf 'cleanup_pending=%s\n' "$cleanup_pending"
  } >"$file" 2>/dev/null || true
}

clear_session_health() {
  local health_file="$1"
  rm -f "$health_file" 2>/dev/null || true
}

cleanup_pending_agent() {
  local health_file="$1" session_file="$2" current_file="$3"
  [ -f "$health_file" ] || return 1
  [ "$(health_field "$health_file" cleanup_pending)" = "1" ] || return 1
  local name team_name current_name
  name="$(sanitize "$(health_field "$health_file" agent)")"
  team_name="$(sanitize "$(health_field "$health_file" team)")"
  [ -n "$name" ] && [ -n "$team_name" ] || return 1
  "$MAGI" agent despawn --team "$team_name" --name "$name" >/dev/null 2>&1 || return 1
  rm -f "$session_file" "$health_file" 2>/dev/null || true
  if [ -n "$current_file" ] && [ -f "$current_file" ]; then
    current_name="$(sed -n '1p' "$current_file" 2>/dev/null || true)"
    [ "$current_name" = "$name" ] && rm -f "$current_file" 2>/dev/null || true
  fi
  return 0
}

record_health_failure() {
  local health_file="$1" name="$2" team_name="$3"
  [ -n "$name" ] && [ -n "$team_name" ] || return 0
  local now failures next_check_at cleanup_pending delay
  now="$(now_seconds)"
  failures="$(health_field "$health_file" failures)"
  next_check_at="$(health_field "$health_file" next_check_at)"
  cleanup_pending="$(health_field "$health_file" cleanup_pending)"
  case "$failures" in ''|*[!0-9]*) failures=0 ;; esac
  case "$next_check_at" in ''|*[!0-9]*) next_check_at=0 ;; esac
  [ "$cleanup_pending" = "1" ] || cleanup_pending=0
  if [ "$cleanup_pending" != "1" ] && [ "$now" -lt "$next_check_at" ]; then
    return 0
  fi
  failures=$((failures + 1))
  case "$failures" in
    1) delay=1 ;;
    2) delay=2 ;;
    *) delay=4 ;;
  esac
  [ "$failures" -ge 3 ] && cleanup_pending=1
  write_health_state "$health_file" "$name" "$team_name" "$failures" "$((now + delay))" "$cleanup_pending"
}

STATE_DIR="${MAGI_CODEX_STATE_DIR:-${XDG_STATE_HOME:-$HOME/.local/state}/magi-codex}"
SESSIONS_DIR="$STATE_DIR/sessions"
CURRENT_DIR="$STATE_DIR/current"
SESSION_ID="$(json_string session_id)"
if [ -z "$SESSION_ID" ]; then
  SESSION_ID="${CODEX_THREAD_ID:-${CODEX_SESSION_ID:-}}"
fi
PROJECT_CWD="$(json_string cwd)"
[ -n "$PROJECT_CWD" ] || PROJECT_CWD="${PWD:-}"

redis_state="unavailable"
agent=""
team=""
if { [ -n "$MAGI" ] && [ -x "$MAGI" ]; }; then
  if "$MAGI" redis status >/dev/null 2>&1; then
    redis_state="reachable"
  else
    redis_state="DOWN"
  fi
  team="$(sanitize "$("$MAGI" config get identity.active_team 2>/dev/null)")"
fi

session_record="missing"
session_team=""
session_file=""
session_key="$(safe_key "$SESSION_ID")"
if [ -n "$session_key" ]; then
  session_file="$SESSIONS_DIR/$session_key.agent"
fi
current_file=""
if [ -n "$PROJECT_CWD" ]; then
  current_file="$CURRENT_DIR/$(safe_key "$PROJECT_CWD").agent"
fi
health_file=""
if [ -n "$session_file" ]; then
  health_file="${session_file%.agent}.health"
fi
if [ -n "$health_file" ] && [ "$redis_state" = "reachable" ]; then
  cleanup_pending_agent "$health_file" "$session_file" "$current_file" || true
fi
if [ -n "$session_key" ]; then
  if [ -f "$session_file" ]; then
    session_record="$(sanitize "$(sed -n '1p' "$session_file" 2>/dev/null)")"
    session_team="$(sanitize "$(sed -n '2p' "$session_file" 2>/dev/null)")"
    [ -n "$session_record" ] || session_record="present"
  fi
fi

if [ "$session_record" = "missing" ] && ephemeral_on \
  && [ "$redis_state" = "reachable" ] && [ -n "$team" ] && [ -n "$session_file" ]; then
  spawned="$(sanitize "$("$MAGI" agent spawn --type codex 2>/dev/null | tail -n1)")"
  if [ -n "$spawned" ]; then
    mkdir -p "$SESSIONS_DIR" 2>/dev/null || true
    printf '%s\n%s\n' "$spawned" "$team" >"$session_file" 2>/dev/null || true
    session_record="$spawned"
    session_team="$team"
  fi
fi

if [ "$session_record" != "missing" ] && [ "$session_record" != "present" ]; then
  agent="$session_record"
fi
if [ -n "$session_team" ]; then
  team="$session_team"
fi

if [ -n "$health_file" ] && [ -f "$session_file" ]; then
  if [ "$redis_state" = "reachable" ]; then
    clear_session_health "$health_file"
  elif [ "$session_record" != "missing" ] && [ "$session_record" != "present" ]; then
    record_health_failure "$health_file" "$session_record" "$session_team"
  fi
fi
if [ -n "$agent" ] && [ -n "$team" ] && [ -n "$current_file" ]; then
  mkdir -p "$CURRENT_DIR" 2>/dev/null || true
  printf '%s\n%s\n' "$agent" "$team" >"$current_file" 2>/dev/null || true
fi

ctx="magi-system context. session_id: ${SESSION_ID:-unset}; agent: ${agent:-unset}; team: ${team:-unset}; redis: ${redis_state}; session_record: ${session_record}; state_dir: $(sanitize "$STATE_DIR")."
printf '{"hookSpecificOutput":{"hookEventName":"UserPromptSubmit","additionalContext":"%s"}}\n' "$ctx"
exit 0
