#!/usr/bin/env bats
#
# Tests for the Codex plugin SessionStart/SessionEnd hooks. HOME and hook state
# are isolated so the real ~/.magi state is never touched.

HOOKS="$BATS_TEST_DIRNAME/../.codex-plugin/hooks"

setup() {
  TEST_HOME="$(mktemp -d)"
  export HOME="$TEST_HOME"
  export MAGI_CODEX_STATE_DIR="$TEST_HOME/state"

  CALLS="$TEST_HOME/calls.log"
  : >"$CALLS"
  REDIS_STATUS_FILE="$TEST_HOME/redis.status"
  printf 'up\n' >"$REDIS_STATUS_FILE"
  CODEX_STATUS_FILE="$TEST_HOME/codex-daemon.status"
  printf 'running\n' >"$CODEX_STATUS_FILE"

  mkdir -p "$TEST_HOME/bin"
  FAKE_CODEX="$TEST_HOME/bin/codex"
  cat >"$FAKE_CODEX" <<EOF
#!/usr/bin/env bash
if [ "\$1 \$2 \$3" = "app-server daemon version" ]; then
  status="\$(cat "$CODEX_STATUS_FILE" 2>/dev/null || printf stopped)"
  printf '{"status":"%s"}\n' "\$status"
  [ "\$status" = "error" ] && exit 1
  exit 0
fi
if [ "\$1 \$2 \$3" = "app-server daemon start" ]; then
  echo "codex-daemon-start \$*" >>"$CALLS"
  printf 'running\n' >"$CODEX_STATUS_FILE"
  printf '{"status":"started"}\n'
  exit 0
fi
exit 0
EOF
  chmod +x "$FAKE_CODEX"
  export PATH="$TEST_HOME/bin:$PATH"

  FAKE_MAGI="$TEST_HOME/magi"
  cat >"$FAKE_MAGI" <<EOF
#!/usr/bin/env bash
if [ "\$1 \$2" = "redis status" ]; then
  [ "\$(cat "$REDIS_STATUS_FILE" 2>/dev/null)" = "down" ] && exit 1
  exit 0
fi
if [ "\$1 \$2" = "redis start" ]; then
  echo "redis-start \$*" >>"$CALLS"
  printf 'up\n' >"$REDIS_STATUS_FILE"
  exit 0
fi
if [ "\$1" = "config" ] && [ "\$2" = "get" ]; then
  case "\$3" in
    identity.active_team) printf 'testteam' ;;
  esac
  exit 0
fi
if [ "\$1" = "config" ] && [ "\$2" = "set" ]; then
  echo "config-set \$*" >>"$CALLS"
  exit 0
fi
if [ "\$1" = "team" ] && [ "\$2" = "create" ]; then
  echo "team-create \$*" >>"$CALLS"
  exit 0
fi
if [ "\$1" = "agent" ] && [ "\$2" = "spawn" ]; then
  echo "spawn \$*" >>"$CALLS"
  printf 'quiet-melchior\n'; exit 0
fi
if [ "\$1" = "agent" ] && [ "\$2" = "name" ]; then
  session="\${CODEX_THREAD_ID:-\${CODEX_SESSION_ID:-}}"
  if [ -n "\$session" ] && [ -f "$MAGI_CODEX_STATE_DIR/sessions/\$session.agent" ]; then
    sed -n '1p' "$MAGI_CODEX_STATE_DIR/sessions/\$session.agent"
  fi
  exit 0
fi
if [ "\$1" = "agent" ] && [ "\$2" = "despawn" ]; then
  echo "despawn \$*" >>"$CALLS"; exit 0
fi
if [ "\$1" = "codex" ] && [ "\$2" = "bridge" ]; then
  echo "bridge \$*" >>"$CALLS"; exit 0
fi
exit 0
EOF
  chmod +x "$FAKE_MAGI"
  export MAGI_BIN="$FAKE_MAGI"
}

teardown() {
  rm -rf "$TEST_HOME"
}

@test "Codex SessionStart spawns a MAGI agent and records it" {
  run bash "$HOOKS/magi-codex-session-start.sh" <<<'{"session_id":"codex-1","cwd":"/tmp/project","hook_event_name":"SessionStart"}'
  [ "$status" -eq 0 ]
  [[ "$output" == *"agent: quiet-melchior"* ]]
  [[ "$output" == *"codex app-server bridge: starting"* ]]

  local file="$MAGI_CODEX_STATE_DIR/sessions/codex-1.agent"
  [ -f "$file" ]
  [ "$(sed -n '1p' "$file")" = "quiet-melchior" ]
  [ "$(sed -n '2p' "$file")" = "testteam" ]
  [ "$(sed -n '3p' "$file")" = "" ]

  local current="$MAGI_CODEX_STATE_DIR/current/tmpproject.agent"
  [ -f "$current" ]
  [ "$(sed -n '1p' "$current")" = "quiet-melchior" ]
  [ "$(sed -n '2p' "$current")" = "testteam" ]
}

@test "Codex SessionStart starts app-server bridge once per session" {
  bash "$HOOKS/magi-codex-session-start.sh" <<<'{"session_id":"codex-bridge","cwd":"/tmp/project","hook_event_name":"SessionStart"}'

  for _ in 1 2 3 4 5; do
    grep -q "bridge codex bridge --thread codex-bridge --cwd /tmp/project --codex codex" "$CALLS" && break
    sleep 0.1
  done
  grep -q "bridge codex bridge --thread codex-bridge --cwd /tmp/project --codex codex" "$CALLS"
  local pid_file="$MAGI_CODEX_STATE_DIR/bridges/codex-bridge.pid"
  [ -f "$pid_file" ]
}

@test "Codex SessionStart starts managed app-server daemon before bridge when stopped" {
  printf 'stopped\n' >"$CODEX_STATUS_FILE"

  run bash "$HOOKS/magi-codex-session-start.sh" <<<'{"session_id":"codex-daemon","cwd":"/tmp/project","hook_event_name":"SessionStart"}'
  [ "$status" -eq 0 ]

  for _ in 1 2 3 4 5; do
    grep -q "bridge codex bridge --thread codex-daemon --cwd /tmp/project --codex codex" "$CALLS" && break
    sleep 0.1
  done
  grep -q '^codex-daemon-start app-server daemon start$' "$CALLS"
  grep -q "bridge codex bridge --thread codex-daemon --cwd /tmp/project --codex codex" "$CALLS"
  local daemon_line bridge_line
  daemon_line="$(grep -n '^codex-daemon-start app-server daemon start$' "$CALLS" | head -1 | cut -d: -f1)"
  bridge_line="$(grep -n 'bridge codex bridge --thread codex-daemon --cwd /tmp/project --codex codex' "$CALLS" | head -1 | cut -d: -f1)"
  [ "$daemon_line" -lt "$bridge_line" ]
}

@test "Codex SessionStart passes explicit app-server socket to bridge" {
  MAGI_CODEX_APP_SERVER_SOCKET=/tmp/codex-app.sock \
    bash "$HOOKS/magi-codex-session-start.sh" <<<'{"session_id":"codex-socket","cwd":"/tmp/project","hook_event_name":"SessionStart"}'

  for _ in 1 2 3 4 5; do
    grep -q "bridge codex bridge --thread codex-socket --cwd /tmp/project --codex codex --socket /tmp/codex-app.sock" "$CALLS" && break
    sleep 0.1
  done
  grep -q "bridge codex bridge --thread codex-socket --cwd /tmp/project --codex codex --socket /tmp/codex-app.sock" "$CALLS"
}

@test "MAGI_CODEX_APP_SERVER_BRIDGE=0 disables Codex bridge startup" {
  MAGI_CODEX_APP_SERVER_BRIDGE=0 run bash "$HOOKS/magi-codex-session-start.sh" <<<'{"session_id":"codex-no-bridge","cwd":"/tmp/project","hook_event_name":"SessionStart"}'
  [ "$status" -eq 0 ]
  [[ "$output" == *"codex app-server bridge: disabled"* ]]
  ! grep -q "bridge codex bridge" "$CALLS"
}

@test "Codex SessionEnd despawns the session agent" {
  bash "$HOOKS/magi-codex-session-start.sh" <<<'{"session_id":"codex-2","cwd":"/tmp/project","hook_event_name":"SessionStart"}'
  run bash "$HOOKS/magi-codex-session-end.sh" <<<'{"session_id":"codex-2","cwd":"/tmp/project","hook_event_name":"SessionEnd"}'
  [ "$status" -eq 0 ]

  [ ! -f "$MAGI_CODEX_STATE_DIR/sessions/codex-2.agent" ]
  grep -q "despawn agent despawn --team testteam --name quiet-melchior" "$CALLS"
}

@test "Codex SessionStart fired twice for one session spawns only once" {
  bash "$HOOKS/magi-codex-session-start.sh" <<<'{"session_id":"codex-3","cwd":"/tmp/project","hook_event_name":"SessionStart"}'
  bash "$HOOKS/magi-codex-session-start.sh" <<<'{"session_id":"codex-3","cwd":"/tmp/project","hook_event_name":"SessionStart"}'
  [ "$(grep -c '^spawn ' "$CALLS")" -eq 1 ]
}

@test "Codex SessionStart falls back to CODEX_THREAD_ID when payload has no session_id" {
  CODEX_THREAD_ID=thread-1 run bash "$HOOKS/magi-codex-session-start.sh" <<<'{"cwd":"/tmp/project","hook_event_name":"SessionStart"}'
  [ "$status" -eq 0 ]
  [[ "$output" == *"agent: quiet-melchior"* ]]

  local file="$MAGI_CODEX_STATE_DIR/sessions/thread-1.agent"
  [ -f "$file" ]
  [ "$(sed -n '1p' "$file")" = "quiet-melchior" ]
}

@test "Codex SessionEnd falls back to CODEX_THREAD_ID when payload has no session_id" {
  CODEX_THREAD_ID=thread-2 bash "$HOOKS/magi-codex-session-start.sh" <<<'{"cwd":"/tmp/project","hook_event_name":"SessionStart"}'
  CODEX_THREAD_ID=thread-2 run bash "$HOOKS/magi-codex-session-end.sh" <<<'{"cwd":"/tmp/project","hook_event_name":"SessionEnd"}'
  [ "$status" -eq 0 ]

  [ ! -f "$MAGI_CODEX_STATE_DIR/sessions/thread-2.agent" ]
  [ ! -f "$MAGI_CODEX_STATE_DIR/bridges/thread-2.pid" ]
  grep -q "despawn agent despawn --team testteam --name quiet-melchior" "$CALLS"
}

@test "Codex UserPromptSubmit injects current magi context" {
  mkdir -p "$MAGI_CODEX_STATE_DIR/sessions"
  printf 'quiet-melchior\ntestteam\n' >"$MAGI_CODEX_STATE_DIR/sessions/thread-3.agent"

  CODEX_THREAD_ID=thread-3 run bash "$HOOKS/magi-codex-prompt-context.sh" <<<'{"cwd":"/tmp/project","hook_event_name":"UserPromptSubmit","user_prompt":"status"}'
  [ "$status" -eq 0 ]
  [[ "$output" == *'"hookEventName":"UserPromptSubmit"'* ]]
  [[ "$output" == *"magi-system context"* ]]
  [[ "$output" == *"session_id: thread-3"* ]]
  [[ "$output" == *"agent: quiet-melchior"* ]]
  [[ "$output" == *"team: testteam"* ]]
  [[ "$output" == *"redis: reachable"* ]]
  [[ "$output" == *"session_record: quiet-melchior"* ]]

  local current="$MAGI_CODEX_STATE_DIR/current/tmpproject.agent"
  [ -f "$current" ]
  [ "$(sed -n '1p' "$current")" = "quiet-melchior" ]
  [ "$(sed -n '2p' "$current")" = "testteam" ]
}

@test "Codex UserPromptSubmit does not run setup.sh for setup MAGI SYSTEM prompt" {
  CODEX_THREAD_ID=thread-setup run bash "$HOOKS/magi-codex-prompt-context.sh" <<<'{"cwd":"/tmp/project","hook_event_name":"UserPromptSubmit","user_prompt":"Setup MAGI SYSTEM"}'
  [ "$status" -eq 0 ]
  [[ "$output" != *"magi-system setup:"* ]]
  ! grep -q '^redis-start redis start$' "$CALLS"
  ! grep -q '^team-create team create testteam$' "$CALLS"
  ! grep -q '^config-set config set identity.active_team testteam$' "$CALLS"
}

@test "Codex UserPromptSubmit reports bridge status sidecar" {
  mkdir -p "$MAGI_CODEX_STATE_DIR/sessions" "$MAGI_CODEX_STATE_DIR/bridges"
  printf 'quiet-melchior\ntestteam\n' >"$MAGI_CODEX_STATE_DIR/sessions/thread-bridge.agent"
  sleep 30 &
  local bridge_pid="$!"
  printf '%s\n' "$bridge_pid" >"$MAGI_CODEX_STATE_DIR/bridges/thread-bridge.pid"
  cat >"$MAGI_CODEX_STATE_DIR/bridges/thread-bridge.status" <<'EOF'
state=retrying
updated_at=100
last_error=failed to connect to socket at /tmp/app-server-control.sock
EOF
  printf 'pid=%s\n' "$bridge_pid" >>"$MAGI_CODEX_STATE_DIR/bridges/thread-bridge.status"

  CODEX_THREAD_ID=thread-bridge run bash "$HOOKS/magi-codex-prompt-context.sh" <<<'{"cwd":"/tmp/project","hook_event_name":"UserPromptSubmit","user_prompt":"status"}'
  kill "$bridge_pid" 2>/dev/null || true
  [ "$status" -eq 0 ]
  [[ "$output" == *"codex app-server bridge: retrying"* ]]
  [[ "$output" == *"last_error: failed to connect to socket at /tmp/app-server-control.sock"* ]]
}

@test "Codex UserPromptSubmit reports unsupported bridge status sidecar" {
  mkdir -p "$MAGI_CODEX_STATE_DIR/sessions" "$MAGI_CODEX_STATE_DIR/bridges"
  printf 'quiet-melchior\ntestteam\n' >"$MAGI_CODEX_STATE_DIR/sessions/thread-unsupported.agent"
  sleep 30 &
  local bridge_pid="$!"
  printf '%s\n' "$bridge_pid" >"$MAGI_CODEX_STATE_DIR/bridges/thread-unsupported.pid"
  cat >"$MAGI_CODEX_STATE_DIR/bridges/thread-unsupported.status" <<'EOF'
state=unsupported
updated_at=100
last_error=Codex app-server control socket not found at /tmp/codex.sock; set MAGI_CODEX_APP_SERVER_SOCKET to a reachable Unix socket.
EOF
  printf 'pid=%s\n' "$bridge_pid" >>"$MAGI_CODEX_STATE_DIR/bridges/thread-unsupported.status"

  CODEX_THREAD_ID=thread-unsupported run bash "$HOOKS/magi-codex-prompt-context.sh" <<<'{"cwd":"/tmp/project","hook_event_name":"UserPromptSubmit","user_prompt":"status"}'
  kill "$bridge_pid" 2>/dev/null || true
  [ "$status" -eq 0 ]
  [[ "$output" == *"codex app-server bridge: unsupported"* ]]
  [[ "$output" == *"last_error: Codex app-server control socket not found at /tmp/codex.sock; set MAGI_CODEX_APP_SERVER_SOCKET to a reachable Unix socket."* ]]
}

@test "Codex UserPromptSubmit cleans stale bridge pid and restarts bridge" {
  mkdir -p "$MAGI_CODEX_STATE_DIR/sessions" "$MAGI_CODEX_STATE_DIR/bridges"
  printf 'quiet-melchior\ntestteam\n' >"$MAGI_CODEX_STATE_DIR/sessions/thread-stale.agent"
  printf '999999\n' >"$MAGI_CODEX_STATE_DIR/bridges/thread-stale.pid"
  cat >"$MAGI_CODEX_STATE_DIR/bridges/thread-stale.status" <<'EOF'
state=running
pid=999999
updated_at=100
last_error=
EOF

  CODEX_THREAD_ID=thread-stale run bash "$HOOKS/magi-codex-prompt-context.sh" <<<'{"cwd":"/tmp/project","hook_event_name":"UserPromptSubmit","user_prompt":"status"}'
  [ "$status" -eq 0 ]
  [[ "$output" == *"codex app-server bridge: starting"* ]]
  grep -q "bridge codex bridge --thread thread-stale --cwd /tmp/project --codex codex" "$CALLS"
  [ -f "$MAGI_CODEX_STATE_DIR/bridges/thread-stale.pid" ]
  [ "$(cat "$MAGI_CODEX_STATE_DIR/bridges/thread-stale.pid")" != "999999" ]
}

@test "Codex UserPromptSubmit starts managed app-server daemon before restarting bridge" {
  mkdir -p "$MAGI_CODEX_STATE_DIR/sessions"
  printf 'quiet-melchior\ntestteam\n' >"$MAGI_CODEX_STATE_DIR/sessions/thread-daemon-prompt.agent"
  printf 'stopped\n' >"$CODEX_STATUS_FILE"

  CODEX_THREAD_ID=thread-daemon-prompt run bash "$HOOKS/magi-codex-prompt-context.sh" <<<'{"cwd":"/tmp/project","hook_event_name":"UserPromptSubmit","user_prompt":"status"}'
  [ "$status" -eq 0 ]

  for _ in 1 2 3 4 5; do
    grep -q "bridge codex bridge --thread thread-daemon-prompt --cwd /tmp/project --codex codex" "$CALLS" && break
    sleep 0.1
  done
  grep -q '^codex-daemon-start app-server daemon start$' "$CALLS"
  grep -q "bridge codex bridge --thread thread-daemon-prompt --cwd /tmp/project --codex codex" "$CALLS"
}

@test "Codex UserPromptSubmit spawns when SessionStart did not record this session" {
  CODEX_THREAD_ID=thread-4 run bash "$HOOKS/magi-codex-prompt-context.sh" <<<'{"cwd":"/tmp/project","hook_event_name":"UserPromptSubmit","user_prompt":"status"}'
  [ "$status" -eq 0 ]
  [[ "$output" == *"session_id: thread-4"* ]]
  [[ "$output" == *"agent: quiet-melchior"* ]]
  [[ "$output" == *"session_record: quiet-melchior"* ]]

  local file="$MAGI_CODEX_STATE_DIR/sessions/thread-4.agent"
  [ -f "$file" ]
  [ "$(sed -n '1p' "$file")" = "quiet-melchior" ]
  [ "$(sed -n '2p' "$file")" = "testteam" ]
  [ "$(sed -n '3p' "$file")" = "" ]
  grep -q "spawn agent spawn --type codex" "$CALLS"
}

@test "MAGI_CODEX_EPHEMERAL=0 disables Codex auto-spawning" {
  MAGI_CODEX_EPHEMERAL=0 run bash "$HOOKS/magi-codex-session-start.sh" <<<'{"session_id":"codex-4","cwd":"/tmp/project","hook_event_name":"SessionStart"}'
  [ "$status" -eq 0 ]
  [ ! -f "$MAGI_CODEX_STATE_DIR/sessions/codex-4.agent" ]
  ! grep -q '^spawn ' "$CALLS"
}

@test "Codex UserPromptSubmit records Redis health failures with nonblocking backoff" {
  mkdir -p "$MAGI_CODEX_STATE_DIR/sessions"
  printf 'quiet-melchior\ntestteam\n' >"$MAGI_CODEX_STATE_DIR/sessions/thread-health.agent"
  printf 'down\n' >"$REDIS_STATUS_FILE"

  MAGI_CODEX_HEALTH_NOW=100 CODEX_THREAD_ID=thread-health run bash "$HOOKS/magi-codex-prompt-context.sh" <<<'{"cwd":"/tmp/project","hook_event_name":"UserPromptSubmit","user_prompt":"status"}'
  [ "$status" -eq 0 ]
  local health="$MAGI_CODEX_STATE_DIR/sessions/thread-health.health"
  [ -f "$health" ]
  grep -q '^failures=1$' "$health"
  grep -q '^next_check_at=101$' "$health"
  grep -q '^cleanup_pending=0$' "$health"

  MAGI_CODEX_HEALTH_NOW=100 CODEX_THREAD_ID=thread-health run bash "$HOOKS/magi-codex-prompt-context.sh" <<<'{"cwd":"/tmp/project","hook_event_name":"UserPromptSubmit","user_prompt":"status"}'
  [ "$status" -eq 0 ]
  grep -q '^failures=1$' "$health"

  MAGI_CODEX_HEALTH_NOW=101 CODEX_THREAD_ID=thread-health run bash "$HOOKS/magi-codex-prompt-context.sh" <<<'{"cwd":"/tmp/project","hook_event_name":"UserPromptSubmit","user_prompt":"status"}'
  [ "$status" -eq 0 ]
  grep -q '^failures=2$' "$health"
  grep -q '^next_check_at=103$' "$health"

  MAGI_CODEX_HEALTH_NOW=103 CODEX_THREAD_ID=thread-health run bash "$HOOKS/magi-codex-prompt-context.sh" <<<'{"cwd":"/tmp/project","hook_event_name":"UserPromptSubmit","user_prompt":"status"}'
  [ "$status" -eq 0 ]
  grep -q '^failures=3$' "$health"
  grep -q '^next_check_at=107$' "$health"
  grep -q '^cleanup_pending=1$' "$health"
  ! grep -q '^despawn ' "$CALLS"
}

@test "Codex UserPromptSubmit despawns cleanup-pending session after Redis recovers" {
  mkdir -p "$MAGI_CODEX_STATE_DIR/sessions" "$MAGI_CODEX_STATE_DIR/current"
  printf 'quiet-melchior\ntestteam\n' >"$MAGI_CODEX_STATE_DIR/sessions/thread-clean.agent"
  printf 'quiet-melchior\ntestteam\n' >"$MAGI_CODEX_STATE_DIR/current/tmpproject.agent"
  cat >"$MAGI_CODEX_STATE_DIR/sessions/thread-clean.health" <<'EOF'
agent=quiet-melchior
team=testteam
failures=3
next_check_at=107
cleanup_pending=1
EOF

  CODEX_THREAD_ID=thread-clean run bash "$HOOKS/magi-codex-prompt-context.sh" <<<'{"cwd":"/tmp/project","hook_event_name":"UserPromptSubmit","user_prompt":"status"}'
  [ "$status" -eq 0 ]
  grep -q "despawn agent despawn --team testteam --name quiet-melchior" "$CALLS"
  [ ! -f "$MAGI_CODEX_STATE_DIR/sessions/thread-clean.health" ]
  [ -f "$MAGI_CODEX_STATE_DIR/current/tmpproject.agent" ]
  [ "$(grep -c '^spawn ' "$CALLS")" -eq 1 ]
}
