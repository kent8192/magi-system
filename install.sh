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

if [ ! -f "$SCRIPT_DIR/Cargo.toml" ] || [ ! -d "$SCRIPT_DIR/templates" ]; then
  if ! command -v git >/dev/null 2>&1; then
    echo "Error: git is required to bootstrap magi from a repository." >&2
    exit 1
  fi

  MAGI_BOOTSTRAP_REPO_URL="${MAGI_BOOTSTRAP_REPO_URL:-https://github.com/kent8192/magi-system.git}"
  BOOTSTRAP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/magi-bootstrap.XXXXXX")"
  cleanup_bootstrap() {
    rm -rf "$BOOTSTRAP_DIR"
  }
  trap cleanup_bootstrap EXIT

  echo "magi installer bootstrap: cloning $MAGI_BOOTSTRAP_REPO_URL"
  git clone --depth 1 "$MAGI_BOOTSTRAP_REPO_URL" "$BOOTSTRAP_DIR/magi"
  MAGI_PLUGIN_REPO="${MAGI_PLUGIN_REPO:-kent8192/magi-system}" "$BOOTSTRAP_DIR/magi/install.sh" "$@"
  exit $?
fi

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
install -m 0755 "$SCRIPT_DIR/setup.sh" "$SKILL_DIR/setup.sh"

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
#   - magi → Claude Code (the event-driven bridge under integrations/)
#   - magi → Codex (the Redis-backed messaging skill at the repo root)
# MAGI_PLUGIN_REPO can point at a GitHub repository for release installs, but
# the default is local so repository checkouts install the manifests being run.
MAGI_PLUGIN_REPO="${MAGI_PLUGIN_REPO:-$SCRIPT_DIR}"
MAGI_PLUGIN_MARKETPLACE="${MAGI_PLUGIN_MARKETPLACE:-magi}"

find_codex_cli() {
  local candidate
  local seen=":"
  while IFS= read -r candidate; do
    case "$seen" in
      *":$candidate:"*) continue ;;
    esac
    seen="$seen$candidate:"
    if "$candidate" plugin --help >/dev/null 2>&1; then
      printf '%s\n' "$candidate"
      return 0
    fi
  done < <(type -P -a codex 2>/dev/null || true)
  return 1
}

claude_plugin_installed() {
  local plugin_id="magi@$MAGI_PLUGIN_MARKETPLACE"
  claude plugin list --json 2>/dev/null | grep -Fq "\"id\": \"$plugin_id\""
}

install_claude_plugin() {
  if ! command -v claude >/dev/null 2>&1; then
    echo "skip: claude CLI not found; not installing the magi Claude Code plugin"
    return 0
  fi
  echo "installing or updating the magi plugin in Claude Code..."
  # Replace the marketplace root first so an existing `magi` marketplace cannot
  # keep pointing at an older checkout while this installer reports success.
  claude plugin marketplace remove "$MAGI_PLUGIN_MARKETPLACE" >/dev/null 2>&1 || true
  claude plugin marketplace add "$MAGI_PLUGIN_REPO" || true
  # Refresh the marketplace before updating so existing plugin installs can pick
  # up the repository's current published version.
  claude plugin marketplace update "$MAGI_PLUGIN_MARKETPLACE" >/dev/null 2>&1 || true
  # Remove the old Claude Code plugin id from pre-rename installs. The plugin's
  # state directory remains unchanged and is not removed here.
  claude plugin uninstall "magi-agent" --scope user --yes >/dev/null 2>&1 || true
  if claude_plugin_installed; then
    if claude plugin update "magi@$MAGI_PLUGIN_MARKETPLACE" --scope user; then
      echo "  updated magi@$MAGI_PLUGIN_MARKETPLACE (restart Claude Code to load it)"
    else
      echo "warning: failed to update magi@$MAGI_PLUGIN_MARKETPLACE in Claude Code" >&2
    fi
  elif claude plugin install "magi@$MAGI_PLUGIN_MARKETPLACE" --scope user; then
    echo "  installed magi@$MAGI_PLUGIN_MARKETPLACE (restart Claude Code to load it)"
  else
    echo "warning: failed to install magi@$MAGI_PLUGIN_MARKETPLACE in Claude Code" >&2
  fi
}

install_codex_plugin() {
  local codex_cli
  if ! codex_cli="$(find_codex_cli)"; then
    if command -v codex >/dev/null 2>&1; then
      echo "skip: codex CLI found but plugin commands are unavailable; not installing the magi Codex plugin"
    else
      echo "skip: codex CLI not found; not installing the magi Codex plugin"
    fi
    return 0
  fi
  echo "installing or updating the magi plugin in Codex..."
  # Codex keeps the existing root when a marketplace name is already present, so
  # replace the source first to ensure this checkout is what gets installed.
  "$codex_cli" plugin marketplace remove "$MAGI_PLUGIN_MARKETPLACE" >/dev/null 2>&1 || true
  "$codex_cli" plugin marketplace add "$MAGI_PLUGIN_REPO" >/dev/null 2>&1 || true
  "$codex_cli" plugin marketplace upgrade "$MAGI_PLUGIN_MARKETPLACE" >/dev/null 2>&1 || true
  "$codex_cli" plugin remove "magi@$MAGI_PLUGIN_MARKETPLACE" >/dev/null 2>&1 || true
  if "$codex_cli" plugin add "magi@$MAGI_PLUGIN_MARKETPLACE" >/dev/null 2>&1; then
    echo "  installed magi@$MAGI_PLUGIN_MARKETPLACE"
  else
    echo "warning: failed to add magi@$MAGI_PLUGIN_MARKETPLACE in Codex" >&2
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
  Claude Code: magi  (restart Claude Code, then run /magi-system setup)
  Codex:       magi

Next:
  1. Run: MAGI_SETUP_TEAM=<team> $SKILL_DIR/setup.sh
  2. In Codex, you can also prompt: \$magi:setup-magi Set up MAGI SYSTEM
  3. Run: ~/.local/bin/magi invite create --team <team>

MSG
