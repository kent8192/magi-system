#!/usr/bin/env bash
set -euo pipefail

# magi — Redis-backed agent messaging installer
#
# Installs:
#   ~/.agents/skills/magi/bin/magi
#   ~/.local/bin/magi
#
# Configuration and Redis state live under:
#   ~/.magi

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
SKILL_DIR="$HOME/.agents/skills/magi"
SKILL_BIN="$SKILL_DIR/bin/magi"
LOCAL_CLI="$HOME/.local/bin/magi"

if ! command -v cargo >/dev/null 2>&1; then
  echo "Error: cargo is required to build magi." >&2
  exit 1
fi

echo "magi — Redis-backed agent messaging"
echo "building release binary..."
# Build into a temporary directory so the repository is not polluted with
# build artifacts; clean it up on exit regardless of success or failure.
BUILD_DIR="$(mktemp -d "${TMPDIR:-/tmp}/magi-build.XXXXXX")"
cleanup() {
  rm -rf "$BUILD_DIR"
}
trap cleanup EXIT
CARGO_TARGET_DIR="$BUILD_DIR" cargo build --release --manifest-path "$SCRIPT_DIR/Cargo.toml"

mkdir -p "$SKILL_DIR/bin" "$SKILL_DIR/templates" "$SKILL_DIR/agents" "$HOME/.local/bin" "$HOME/.magi"
install -m 0755 "$BUILD_DIR/release/magi" "$SKILL_BIN"
install -m 0755 "$BUILD_DIR/release/magi" "$LOCAL_CLI"

sed "s/__SKILL_NAME__/magi/g" "$SCRIPT_DIR/templates/cmd.codex.md" > "$SKILL_DIR/SKILL.md"
for tmpl in "$SCRIPT_DIR/templates/"cmd.*.md; do
  sed "s/__SKILL_NAME__/magi/g" "$tmpl" > "$SKILL_DIR/templates/$(basename "$tmpl")"
done
if [ -f "$SCRIPT_DIR/openai.yaml" ]; then
  cp "$SCRIPT_DIR/openai.yaml" "$SKILL_DIR/agents/openai.yaml"
fi

if [ -d "$HOME/.claude" ]; then
  mkdir -p "$HOME/.claude/commands"
  sed "s/__SKILL_NAME__/magi/g" "$SCRIPT_DIR/templates/cmd.claude-code.md" > "$HOME/.claude/commands/magi.md"
fi

"$LOCAL_CLI" install

# --- Install the magi plugins into Claude Code and Codex (best effort) ---
# Both CLIs resolve the "magi-dev" marketplace from this repository's root
# manifest (.claude-plugin/marketplace.json) and install the plugin that
# targets their runtime:
#   - magi-agent → Claude Code (the event-driven bridge under integrations/)
#   - magi       → Codex (the Redis-backed messaging skill at the repo root)
# The marketplace is fetched from the GitHub repository, so the plugins must be
# published on the repository's default branch. Each step is best effort: if the
# CLI is absent or a command fails, the magi binary install above is unaffected.
MAGI_PLUGIN_REPO="${MAGI_PLUGIN_REPO:-kent8192/magi}"
MAGI_PLUGIN_MARKETPLACE="${MAGI_PLUGIN_MARKETPLACE:-magi-dev}"

install_claude_plugin() {
  if ! command -v claude >/dev/null 2>&1; then
    echo "skip: claude CLI not found; not installing the magi-agent Claude Code plugin"
    return 0
  fi
  echo "installing the magi-agent plugin into Claude Code..."
  # Adding a marketplace that already exists is treated as success.
  claude plugin marketplace add "$MAGI_PLUGIN_REPO" || true
  if claude plugin install "magi-agent@$MAGI_PLUGIN_MARKETPLACE" --scope user; then
    echo "  installed magi-agent@$MAGI_PLUGIN_MARKETPLACE (restart Claude Code to load it)"
  else
    echo "warning: failed to install magi-agent@$MAGI_PLUGIN_MARKETPLACE into Claude Code" >&2
  fi
}

install_codex_plugin() {
  if ! command -v codex >/dev/null 2>&1; then
    echo "skip: codex CLI not found; not installing the magi Codex plugin"
    return 0
  fi
  echo "installing the magi plugin into Codex..."
  codex plugin marketplace add "$MAGI_PLUGIN_REPO" || true
  if codex plugin add "magi@$MAGI_PLUGIN_MARKETPLACE"; then
    echo "  installed magi@$MAGI_PLUGIN_MARKETPLACE"
  else
    echo "warning: failed to add magi@$MAGI_PLUGIN_MARKETPLACE into Codex" >&2
  fi
}

install_claude_plugin
install_codex_plugin

cat <<MSG

Installed magi:
  $SKILL_BIN
  $LOCAL_CLI

Configuration:
  $HOME/.magi

Plugins (best effort, from the "$MAGI_PLUGIN_MARKETPLACE" marketplace):
  Claude Code: magi-agent  (restart Claude Code, then run /magi-system setup)
  Codex:       magi

Next:
  1. Run: ~/.local/bin/magi redis start
  2. Run: ~/.local/bin/magi team create <team>
  3. Run: ~/.local/bin/magi invite create --team <team>

MSG
