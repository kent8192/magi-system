#!/usr/bin/env bash
#
# Keeps the managed Codex app-server daemon reachable for magi live delivery.
# This hook is intentionally quiet: it repairs daemon availability when possible
# without adding prompt context or failing the Codex hook phase.

set -euo pipefail

daemon_auto_on() {
  case "$(printf '%s' "${MAGI_CODEX_APP_SERVER_DAEMON:-1}" | tr '[:upper:]' '[:lower:]')" in
    0 | false | no | off) return 1 ;;
    *) return 0 ;;
  esac
}

codex_cmd_available() {
  local codex_cli="${MAGI_CODEX_CLI:-codex}"
  if [ -n "${MAGI_CODEX_CLI_SHELL:-}" ]; then
    [ -x "$codex_cli" ]
  else
    command -v "$codex_cli" >/dev/null 2>&1
  fi
}

codex_daemon_running() {
  local codex_cli="${MAGI_CODEX_CLI:-codex}"
  local status
  if [ -n "${MAGI_CODEX_CLI_SHELL:-}" ]; then
    status="$("$MAGI_CODEX_CLI_SHELL" "$codex_cli" app-server daemon version 2>/dev/null)" || return 1
  else
    status="$("$codex_cli" app-server daemon version 2>/dev/null)" || return 1
  fi
  printf '%s\n' "$status" | grep -q '"status"[[:space:]]*:[[:space:]]*"running"'
}

codex_daemon_command() {
  local command="$1"
  local codex_cli="${MAGI_CODEX_CLI:-codex}"
  if [ -n "${MAGI_CODEX_CLI_SHELL:-}" ]; then
    "$MAGI_CODEX_CLI_SHELL" "$codex_cli" app-server daemon "$command" >/dev/null 2>&1 || true
  else
    "$codex_cli" app-server daemon "$command" >/dev/null 2>&1 || true
  fi
}

ensure_codex_daemon() {
  [ -z "${MAGI_CODEX_APP_SERVER_SOCKET:-}" ] || return 0
  daemon_auto_on || return 0
  codex_cmd_available || return 0
  codex_daemon_running && return 0
  codex_daemon_command start
  codex_daemon_running && return 0
  codex_daemon_command restart
  codex_daemon_running || true
}

ensure_codex_daemon
exit 0
