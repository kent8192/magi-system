#!/usr/bin/env bash
set -euo pipefail

# MAGI SYSTEM setup entrypoint for installed hooks and manual setup prompts.

MAGI="${MAGI_BIN:-}"
[ -n "$MAGI" ] || MAGI="$(command -v magi 2>/dev/null || true)"
if [ -z "$MAGI" ]; then
  for candidate in "$HOME/.agents/skills/magi/bin/magi" "$HOME/.local/bin/magi"; do
    [ -x "$candidate" ] && MAGI="$candidate" && break
  done
fi

if [ -z "$MAGI" ] || [ ! -x "$MAGI" ]; then
  echo "Error: magi CLI not found. Run ./install.sh first." >&2
  exit 1
fi

magi() {
  "$MAGI" "$@"
}

active_team="$(magi config get identity.active_team 2>/dev/null | sed -n '1p' || true)"
team="${MAGI_SETUP_TEAM:-${MAGI_TEAM:-$active_team}}"
[ -n "$team" ] || team="magi"

magi redis start

if ! magi team list 2>/dev/null | grep -Fxq "$team"; then
  create_output="$(magi team create "$team" 2>&1)" || {
    if ! printf '%s' "$create_output" | grep -Fq "already exists"; then
      printf '%s\n' "$create_output" >&2
      exit 1
    fi
  }
fi

magi config set identity.active_team "$team"

printf 'magi-system setup: ok; team: %s\n' "$team"
