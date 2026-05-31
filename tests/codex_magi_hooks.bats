#!/usr/bin/env bats
#
# Tests for the Codex plugin SessionStart/SessionEnd hooks. HOME and hook state
# are isolated so the real ~/.magi state is never touched.

HOOKS="$BATS_TEST_DIRNAME/../.codex-plugin/hooks"

setup() {
  TEST_HOME="$(mktemp -d)"
  export HOME="$TEST_HOME"
  export MAGI_CODEX_STATE_DIR="$TEST_HOME/state"

  ACTIVE_AGENT_FILE="$TEST_HOME/active_agent"
  printf 'kent8192' >"$ACTIVE_AGENT_FILE"
  CALLS="$TEST_HOME/calls.log"
  : >"$CALLS"

  FAKE_MAGI="$TEST_HOME/magi"
  cat >"$FAKE_MAGI" <<EOF
#!/usr/bin/env bash
[ "\$1 \$2" = "redis status" ] && exit 0
if [ "\$1" = "config" ] && [ "\$2" = "get" ]; then
  case "\$3" in
    identity.active_agent) cat "$ACTIVE_AGENT_FILE" 2>/dev/null ;;
    identity.active_team) printf 'testteam' ;;
  esac
  exit 0
fi
if [ "\$1" = "config" ] && [ "\$2" = "set" ] && [ "\$3" = "identity.active_agent" ]; then
  printf '%s' "\$4" >"$ACTIVE_AGENT_FILE"; exit 0
fi
if [ "\$1" = "agent" ] && [ "\$2" = "spawn" ]; then
  echo "spawn \$*" >>"$CALLS"
  printf '%s' "quiet-melchior" >"$ACTIVE_AGENT_FILE"
  printf 'quiet-melchior\n'; exit 0
fi
if [ "\$1" = "agent" ] && [ "\$2" = "despawn" ]; then
  echo "despawn \$*" >>"$CALLS"; exit 0
fi
exit 0
EOF
  chmod +x "$FAKE_MAGI"
  export MAGI_BIN="$FAKE_MAGI"
}

teardown() {
  rm -rf "$TEST_HOME"
}

@test "Codex SessionStart spawns a MAGI agent and adopts the identity" {
  run bash "$HOOKS/magi-codex-session-start.sh" <<<'{"session_id":"codex-1","cwd":"/tmp/project","hook_event_name":"SessionStart"}'
  [ "$status" -eq 0 ]
  [[ "$output" == *"agent: quiet-melchior"* ]]

  local file="$MAGI_CODEX_STATE_DIR/sessions/codex-1.agent"
  [ -f "$file" ]
  [ "$(sed -n '1p' "$file")" = "quiet-melchior" ]
  [ "$(sed -n '2p' "$file")" = "testteam" ]
  [ "$(sed -n '3p' "$file")" = "kent8192" ]
  [ "$(cat "$ACTIVE_AGENT_FILE")" = "quiet-melchior" ]
}

@test "Codex SessionEnd despawns the session agent and restores identity" {
  bash "$HOOKS/magi-codex-session-start.sh" <<<'{"session_id":"codex-2","cwd":"/tmp/project","hook_event_name":"SessionStart"}'
  run bash "$HOOKS/magi-codex-session-end.sh" <<<'{"session_id":"codex-2","cwd":"/tmp/project","hook_event_name":"SessionEnd"}'
  [ "$status" -eq 0 ]

  [ ! -f "$MAGI_CODEX_STATE_DIR/sessions/codex-2.agent" ]
  grep -q "despawn agent despawn --team testteam --name quiet-melchior" "$CALLS"
  [ "$(cat "$ACTIVE_AGENT_FILE")" = "kent8192" ]
}

@test "Codex SessionStart fired twice for one session spawns only once" {
  bash "$HOOKS/magi-codex-session-start.sh" <<<'{"session_id":"codex-3","cwd":"/tmp/project","hook_event_name":"SessionStart"}'
  bash "$HOOKS/magi-codex-session-start.sh" <<<'{"session_id":"codex-3","cwd":"/tmp/project","hook_event_name":"SessionStart"}'
  [ "$(grep -c '^spawn ' "$CALLS")" -eq 1 ]
}

@test "MAGI_CODEX_EPHEMERAL=0 disables Codex auto-spawning" {
  MAGI_CODEX_EPHEMERAL=0 run bash "$HOOKS/magi-codex-session-start.sh" <<<'{"session_id":"codex-4","cwd":"/tmp/project","hook_event_name":"SessionStart"}'
  [ "$status" -eq 0 ]
  [ ! -f "$MAGI_CODEX_STATE_DIR/sessions/codex-4.agent" ]
  ! grep -q '^spawn ' "$CALLS"
}
