#!/usr/bin/env bats
#
# Tests for the magi-agent plugin SessionStart/SessionEnd hooks. A fake `magi`
# binary stands in for the real CLI so the ephemeral-agent lifecycle (spawn ->
# record -> despawn) can be asserted without Redis or Claude
# Code. HOME and the daemon state dir are isolated so the real ~/.magi and
# ~/.local/state are never touched.

HOOKS="$BATS_TEST_DIRNAME/../integrations/magi-agent-plugin/hooks"

setup() {
  TEST_HOME="$(mktemp -d)"
  export HOME="$TEST_HOME"
  export MAGI_AGENT_STATE_DIR="$TEST_HOME/state"

  CALLS="$TEST_HOME/calls.log"
  : >"$CALLS"

  # Fake magi: reachable Redis, a configured team, and spawn/despawn that record
  # their invocation like the real CLI.
  FAKE_MAGI="$TEST_HOME/magi"
  cat >"$FAKE_MAGI" <<EOF
#!/usr/bin/env bash
[ "\$1 \$2" = "redis status" ] && exit 0
if [ "\$1" = "config" ] && [ "\$2" = "get" ]; then
  case "\$3" in
    identity.active_team) printf 'testteam' ;;
  esac
  exit 0
fi
if [ "\$1" = "agent" ] && [ "\$2" = "spawn" ]; then
  echo "spawn \$*" >>"$CALLS"
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

@test "SessionStart spawns a MAGI agent and records it" {
  run bash "$HOOKS/magi-session-start.sh" <<<'{"session_id":"sess-1","source":"startup"}'
  [ "$status" -eq 0 ]
  # Context reflects the freshly spawned agent.
  [[ "$output" == *"agent: quiet-melchior"* ]]

  local file="$MAGI_AGENT_STATE_DIR/sessions/sess-1.agent"
  [ -f "$file" ]
  [ "$(sed -n '1p' "$file")" = "quiet-melchior" ]
  [ "$(sed -n '2p' "$file")" = "testteam" ]
  [ "$(sed -n '3p' "$file")" = "" ]
}

@test "SessionEnd despawns the agent and clears the record" {
  bash "$HOOKS/magi-session-start.sh" <<<'{"session_id":"sess-2","source":"startup"}'
  run bash "$HOOKS/magi-session-end.sh" <<<'{"session_id":"sess-2","reason":"exit"}'
  [ "$status" -eq 0 ]

  [ ! -f "$MAGI_AGENT_STATE_DIR/sessions/sess-2.agent" ]
  grep -q "despawn agent despawn --team testteam --name quiet-melchior" "$CALLS"
}

@test "SessionStart fired twice for one session spawns only once" {
  bash "$HOOKS/magi-session-start.sh" <<<'{"session_id":"sess-3","source":"startup"}'
  bash "$HOOKS/magi-session-start.sh" <<<'{"session_id":"sess-3","source":"startup"}'
  [ "$(grep -c '^spawn ' "$CALLS")" -eq 1 ]
}

@test "MAGI_AGENT_EPHEMERAL=0 disables spawning" {
  MAGI_AGENT_EPHEMERAL=0 run bash "$HOOKS/magi-session-start.sh" <<<'{"session_id":"sess-4","source":"startup"}'
  [ "$status" -eq 0 ]
  [ ! -f "$MAGI_AGENT_STATE_DIR/sessions/sess-4.agent" ]
  ! grep -q '^spawn ' "$CALLS"
}

@test "SessionEnd with no recorded session is a no-op" {
  run bash "$HOOKS/magi-session-end.sh" <<<'{"session_id":"never-started","reason":"exit"}'
  [ "$status" -eq 0 ]
  [ ! -s "$CALLS" ]
}
