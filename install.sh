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
# Both CLIs resolve the "magi" marketplace from this checkout and install the
# plugin that targets their runtime:
#   - magi-agent → Claude Code (the event-driven bridge under integrations/)
#   - magi       → Codex (the Redis-backed messaging skill at the repo root)
# MAGI_PLUGIN_REPO can point at a GitHub repository for release installs, but
# the default is local so repository checkouts install the manifests being run.
MAGI_PLUGIN_REPO="${MAGI_PLUGIN_REPO:-$SCRIPT_DIR}"
MAGI_PLUGIN_MARKETPLACE="${MAGI_PLUGIN_MARKETPLACE:-magi}"

install_claude_plugin() {
  if ! command -v claude >/dev/null 2>&1; then
    echo "skip: claude CLI not found; not installing the magi-agent Claude Code plugin"
    return 0
  fi
  echo "installing or updating the magi-agent plugin in Claude Code..."
  # Adding a marketplace that already exists is treated as success.
  claude plugin marketplace add "$MAGI_PLUGIN_REPO" || true
  # Refresh the marketplace before updating so existing plugin installs can pick
  # up the repository's current published version.
  claude plugin marketplace update "$MAGI_PLUGIN_MARKETPLACE" >/dev/null 2>&1 || true
  if claude plugin update "magi-agent" --scope user; then
    echo "  updated magi-agent (restart Claude Code to load it)"
  elif claude plugin install "magi-agent@$MAGI_PLUGIN_MARKETPLACE" --scope user; then
    echo "  installed magi-agent@$MAGI_PLUGIN_MARKETPLACE (restart Claude Code to load it)"
  else
    echo "warning: failed to install or update magi-agent@$MAGI_PLUGIN_MARKETPLACE in Claude Code" >&2
  fi
}

install_codex_plugin() {
  if ! command -v codex >/dev/null 2>&1; then
    echo "skip: codex CLI not found; not installing the magi Codex plugin"
    return 0
  fi
  echo "installing or updating the magi plugin in Codex..."
  codex plugin marketplace add "$MAGI_PLUGIN_REPO" || true
  codex plugin marketplace update "$MAGI_PLUGIN_MARKETPLACE" >/dev/null 2>&1 || true
  if codex plugin update "magi@$MAGI_PLUGIN_MARKETPLACE"; then
    echo "  updated magi@$MAGI_PLUGIN_MARKETPLACE"
  elif codex plugin update "magi"; then
    echo "  updated magi"
  elif codex plugin add "magi@$MAGI_PLUGIN_MARKETPLACE"; then
    echo "  installed magi@$MAGI_PLUGIN_MARKETPLACE"
  else
    echo "warning: failed to add or update magi@$MAGI_PLUGIN_MARKETPLACE in Codex" >&2
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
