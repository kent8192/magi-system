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
  export MAGI_AGENT_AUTOSTART_REDIS=0
  export MAGI_AGENT_AUTOSTART_BRIDGE=0
  unset BASH_ENV
  unset ENV
  unset PYTHONSTARTUP

  CALLS="$TEST_HOME/calls.log"
  : >"$CALLS"

  # Fake magi: reachable Redis, a configured team, and spawn/despawn that record
  # their invocation like the real CLI.
  FAKE_MAGI="$TEST_HOME/magi"
  cat >"$FAKE_MAGI" <<EOF
#!/bin/sh
if [ "\$1 \$2" = "redis status" ]; then
  [ "\${MAGI_FAKE_REDIS_DOWN:-0}" = "1" ] && exit 1
  exit 0
fi
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
if [ "\$1" = "watch" ]; then
  echo "watch \$*" >>"$CALLS"
  if [ -n "\${MAGI_FAKE_WATCH_SLEEP:-}" ]; then
    sleep "\$MAGI_FAKE_WATCH_SLEEP"
  fi
  printf 'fatherly-balthasar->quiet-melchior: hello from redis\n'
  exit 0
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
  [[ "$output" == *"Claude Code Monitor directive"* ]]
  [[ "$output" == *"magi-monitor-once.sh sess-1"* ]]

  local file="$MAGI_AGENT_STATE_DIR/sessions/sess-1.agent"
  [ -f "$file" ]
  [ "$(sed -n '1p' "$file")" = "quiet-melchior" ]
  [ "$(sed -n '2p' "$file")" = "testteam" ]
  [ "$(sed -n '3p' "$file")" = "" ]
}

@test "SessionStart directs Monitor even when bridge is running" {
  mkdir -p "$MAGI_AGENT_STATE_DIR"
  sleep 10 &
  local bridge_pid="$!"
  printf '%s\n' "$bridge_pid" >"$MAGI_AGENT_STATE_DIR/agentd.pid"

  run bash "$HOOKS/magi-session-start.sh" <<<'{"session_id":"sess-bridge","source":"startup"}'
  kill "$bridge_pid" 2>/dev/null || true
  wait "$bridge_pid" 2>/dev/null || true

  [ "$status" -eq 0 ]
  [[ "$output" == *"auto-reply bridge: running"* ]]
  [[ "$output" == *"Claude Code Monitor directive"* ]]
  [[ "$output" == *"magi-monitor-once.sh sess-bridge"* ]]
}

@test "magi Monitor wrapper emits context-form messages and uses the session id" {
  run bash "$HOOKS/magi-monitor-once.sh" "sess-monitor"
  [ "$status" -eq 0 ]
  [ "$output" = "fatherly-balthasar->quiet-melchior: hello from redis" ]
  grep -q "watch watch --once --format context" "$CALLS"
}

@test "magi Monitor wrapper records and clears a session pid" {
  MAGI_FAKE_WATCH_SLEEP=1 "$HOOKS/magi-monitor-once.sh" "sess-pid" >"$TEST_HOME/monitor.out" &
  local monitor_pid="$!"
  local pid_file="$MAGI_AGENT_STATE_DIR/monitors/sess-pid.pid"

  for _ in 1 2 3 4 5 6 7 8 9 10; do
    [ -f "$pid_file" ] && break
    sleep 0.1
  done

  [ -f "$pid_file" ]
  [ "$(cat "$pid_file")" = "$monitor_pid" ]
  wait "$monitor_pid"
  [ ! -f "$pid_file" ]
  [ "$(cat "$TEST_HOME/monitor.out")" = "fatherly-balthasar->quiet-melchior: hello from redis" ]
}

@test "Monitor ensure hook directs Monitor when no session pid is running" {
  mkdir -p "$MAGI_AGENT_STATE_DIR/sessions"
  printf 'quiet-melchior\ntestteam\n' >"$MAGI_AGENT_STATE_DIR/sessions/sess-ensure.agent"

  run bash "$HOOKS/magi-monitor-ensure.sh" <<<'{"session_id":"sess-ensure","hook_event_name":"UserPromptSubmit"}'
  [ "$status" -eq 0 ]
  [[ "$output" == *'"systemMessage"'* ]]
  [[ "$output" == *"Claude Code Monitor directive"* ]]
  [[ "$output" == *"magi-monitor-once.sh sess-ensure"* ]]
}

@test "Monitor ensure hook skips duplicate directive while session pid is running" {
  mkdir -p "$MAGI_AGENT_STATE_DIR/sessions" "$MAGI_AGENT_STATE_DIR/monitors"
  printf 'quiet-melchior\ntestteam\n' >"$MAGI_AGENT_STATE_DIR/sessions/sess-running.agent"
  sleep 10 &
  local running_pid="$!"
  printf '%s\n' "$running_pid" >"$MAGI_AGENT_STATE_DIR/monitors/sess-running.pid"

  run bash "$HOOKS/magi-monitor-ensure.sh" <<<'{"session_id":"sess-running","hook_event_name":"PostToolUse"}'
  kill "$running_pid" 2>/dev/null || true
  wait "$running_pid" 2>/dev/null || true

  [ "$status" -eq 0 ]
  [[ "$output" == *"Monitor already running"* ]]
  [[ "$output" != *"magi-monitor-once.sh sess-running"* ]]
}

@test "Monitor ensure hook replaces stale pid with a new directive" {
  mkdir -p "$MAGI_AGENT_STATE_DIR/sessions" "$MAGI_AGENT_STATE_DIR/monitors"
  printf 'quiet-melchior\ntestteam\n' >"$MAGI_AGENT_STATE_DIR/sessions/sess-stale.agent"
  printf '999999\n' >"$MAGI_AGENT_STATE_DIR/monitors/sess-stale.pid"

  run bash "$HOOKS/magi-monitor-ensure.sh" <<<'{"session_id":"sess-stale","hook_event_name":"Stop"}'
  [ "$status" -eq 0 ]
  [[ "$output" == *"Claude Code Monitor directive"* ]]
  [[ "$output" == *"magi-monitor-once.sh sess-stale"* ]]
  [ ! -f "$MAGI_AGENT_STATE_DIR/monitors/sess-stale.pid" ]
}

@test "Claude Code hooks register Monitor ensure phases" {
  grep -q '"UserPromptSubmit"' "$HOOKS/hooks.json"
  grep -q '"PostToolUse"' "$HOOKS/hooks.json"
  grep -q '"Stop"' "$HOOKS/hooks.json"
  [ "$(grep -c 'magi-monitor-ensure.sh' "$HOOKS/hooks.json")" -eq 3 ]
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

@test "SessionStart records Redis health failures with nonblocking backoff" {
  mkdir -p "$MAGI_AGENT_STATE_DIR/sessions"
  printf 'quiet-melchior\ntestteam\n' >"$MAGI_AGENT_STATE_DIR/sessions/sess-health.agent"

  MAGI_FAKE_REDIS_DOWN=1 MAGI_AGENT_HEALTH_NOW=100 run bash "$HOOKS/magi-session-start.sh" <<<'{"session_id":"sess-health","source":"startup"}'
  [ "$status" -eq 0 ]
  local health="$MAGI_AGENT_STATE_DIR/sessions/sess-health.health"
  [ -f "$health" ]
  grep -q '^failures=1$' "$health"
  grep -q '^next_check_at=101$' "$health"
  grep -q '^cleanup_pending=0$' "$health"

  MAGI_FAKE_REDIS_DOWN=1 MAGI_AGENT_HEALTH_NOW=100 run bash "$HOOKS/magi-session-start.sh" <<<'{"session_id":"sess-health","source":"startup"}'
  [ "$status" -eq 0 ]
  grep -q '^failures=1$' "$health"

  MAGI_FAKE_REDIS_DOWN=1 MAGI_AGENT_HEALTH_NOW=101 run bash "$HOOKS/magi-session-start.sh" <<<'{"session_id":"sess-health","source":"startup"}'
  [ "$status" -eq 0 ]
  grep -q '^failures=2$' "$health"
  grep -q '^next_check_at=103$' "$health"

  MAGI_FAKE_REDIS_DOWN=1 MAGI_AGENT_HEALTH_NOW=103 run bash "$HOOKS/magi-session-start.sh" <<<'{"session_id":"sess-health","source":"startup"}'
  [ "$status" -eq 0 ]
  grep -q '^failures=3$' "$health"
  grep -q '^next_check_at=107$' "$health"
  grep -q '^cleanup_pending=1$' "$health"
  ! grep -q '^despawn ' "$CALLS"
}

@test "SessionStart despawns cleanup-pending session after Redis recovers" {
  mkdir -p "$MAGI_AGENT_STATE_DIR/sessions"
  printf 'quiet-melchior\ntestteam\n' >"$MAGI_AGENT_STATE_DIR/sessions/sess-clean.agent"
  cat >"$MAGI_AGENT_STATE_DIR/sessions/sess-clean.health" <<'EOF'
agent=quiet-melchior
team=testteam
failures=3
next_check_at=107
cleanup_pending=1
EOF

  run bash "$HOOKS/magi-session-start.sh" <<<'{"session_id":"sess-clean","source":"startup"}'
  [ "$status" -eq 0 ]
  grep -q "despawn agent despawn --team testteam --name quiet-melchior" "$CALLS"
  [ ! -f "$MAGI_AGENT_STATE_DIR/sessions/sess-clean.health" ]
  [ "$(grep -c '^spawn ' "$CALLS")" -eq 1 ]
}
