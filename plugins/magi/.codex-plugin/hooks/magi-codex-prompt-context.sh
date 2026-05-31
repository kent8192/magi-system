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
current_file=""
if [ -n "$PROJECT_CWD" ]; then
  current_file="$CURRENT_DIR/$(safe_key "$PROJECT_CWD").agent"
fi
if [ -n "$agent" ] && [ -n "$team" ] && [ -n "$current_file" ]; then
  mkdir -p "$CURRENT_DIR" 2>/dev/null || true
  printf '%s\n%s\n' "$agent" "$team" >"$current_file" 2>/dev/null || true
fi

ctx="magi-system context. session_id: ${SESSION_ID:-unset}; agent: ${agent:-unset}; team: ${team:-unset}; redis: ${redis_state}; session_record: ${session_record}; state_dir: $(sanitize "$STATE_DIR")."
printf '{"hookSpecificOutput":{"hookEventName":"UserPromptSubmit","additionalContext":"%s"}}\n' "$ctx"
exit 0
