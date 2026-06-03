<div align="center">
  <p>
    <img src="docs/screen-record.gif" alt="MAGI SYSTEM screen recording" width="720"/>
  </p>

  <p>
    <img src="branding/icon.png" alt="MAGI SYSTEM icon" width="200"/>
  </p>

  <h1>MAGI SYSTEM</h1>

  <h3>Redis-backed cross-agent messaging for CLI AI agents</h3>

  <p><strong>Durable team messaging for Codex, Claude Code, and other CLI agents</strong></p>
  <p>Coordinate agents through Redis Streams,<br/>
  Pub/Sub wakeups, team invites, and session-scoped inboxes.</p>
</div>

---

magi is a Rust CLI that stores team membership, invites, message history, and
per-agent inbox cursors in Redis. Redis Streams are the durable message log and
Pub/Sub is used as a low-latency wakeup for `watch`.

## Install

```bash
curl -fsSL https://raw.githubusercontent.com/kent8192/magi-system/main/install.sh | bash
```

The standalone installer only needs a shell, `git`, and the Rust toolchain. It
clones `https://github.com/kent8192/magi-system.git` into a temporary directory,
builds magi from that checkout, and places the binary at:

- `~/.agents/skills/magi/bin/magi`
- `~/.local/bin/magi`

Configuration and managed Redis state are stored under `~/.magi`.

### Codex runtime requirements

The Codex plugin and live app-server bridge are currently verified with Codex
Standalone. The Claude Code plugin and bridge are documented, but they have not
yet been verified against the current Claude Code runtime.

The documented Codex workflow requires Docker. The general `magi` Redis
lifecycle can fall back to a local `redis-server`, but Codex Standalone,
managed Redis, and Dev Container usage are tested against the Docker-backed
path.

Codex hook execution must be enabled for the plugin to create and clean up the
session-scoped magi agent, inject prompt context, and start the app-server
bridge. Ensure the Codex runtime has hook support enabled, including
`hooks` and `plugin_hooks` feature flags when your Codex build exposes them:

```toml
[features]
hooks = true
plugin_hooks = true
```

The repository devcontainer mounts the Docker socket plus the host MAGI and
Codex runtime state needed for immediate Codex injection:

- host `~/.magi` as container `/home/vscode/.magi`
- host `~/.local/state/magi-codex` as container
  `/home/vscode/.local/state/magi-codex`
- host `~/.codex/app-server-control` as container
  `/home/vscode/.codex/app-server-control`

It also sets `MAGI_CODEX_STATE_DIR` to the container state path above. With
those mounts, `magi codex bridge` in the devcontainer uses the same Redis
credentials, session records, and Codex app-server control socket as the host
runtime. If the mounts are absent or the host Codex app-server daemon is not
running, bridge delivery remains safe: the bridge reports `unsupported` with
the app-server socket path instead of advancing the inbox cursor.

Override the bootstrap source with `MAGI_BOOTSTRAP_REPO_URL`:

```bash
curl -fsSL https://raw.githubusercontent.com/kent8192/magi-system/main/install.sh \
  | MAGI_BOOTSTRAP_REPO_URL=https://github.com/kent8192/magi-system.git bash
```

From a local checkout, run `./install.sh` directly to install the code and
plugin manifests from that checkout instead of bootstrapping a temporary clone.

The installer then registers or updates the magi plugins (best effort) with
whichever agent CLIs are present — `claude` and `codex` — and `./uninstall.sh`
removes them again. See [Plugins](#plugins) below. Override the source
repository or marketplace name with the `MAGI_PLUGIN_REPO` and
`MAGI_PLUGIN_MARKETPLACE` environment variables.

## Plugins

The repository ships two plugins under the `magi` marketplace:

- **`magi` (Codex)** — manifest at `.codex-plugin/plugin.json`, mirrored into
  the `plugins/magi/` marketplace package root for installation. The package
  keeps root-level `plugin.json`, `hooks/`, and `skills/` copies because Codex
  resolves plugin resources from the installed plugin root. It exposes the
  `magi` messaging skill plus Codex session hooks that spawn a session-scoped
  `codex` agent, inject magi-system context on each prompt, self-heal missing or
  stale session records, and clean the agent up on session end or after repeated
  Redis health-check failures. Hook context only reports an `agent` value after
  Redis confirms that name is still a member of the active team. The package also
  exposes the `setup-magi` skill for setup prompts; that skill reads `setup.sh`
  and follows the entrypoint that starts managed Redis, creates the active setup
  team when missing, and stores it through the `magi` CLI. The hooks also ensure
  the managed Codex app-server daemon is running, then launch
  `magi codex bridge` for the current Codex thread so Redis Pub/Sub wakeups
  are first injected into Codex thread history over the Unix control socket's
  WebSocket transport, then followed by a best-effort Codex app-server turn.
  Prompt hooks report the bridge state on each prompt. App-server injection
  failures keep the bridge in `retrying` until a later injection succeeds; Codex
  runtimes without a reachable app-server control socket are reported as
  `unsupported`.
- **`magi` (Claude Code)** — the event-driven bridge under
  `integrations/magi-agent-plugin/` that turns incoming magi messages into a
  live Claude session. Claude Code hooks also keep at least one Monitor job
  waiting for the session inbox by asking Claude Code to run
  `magi watch --once --format context` whenever no live Monitor pid is recorded.
  The Stop hook also checks Claude Code's `background_tasks` registry and blocks
  stopping when no `monitor` task is present. Redis wakeups surface as
  `<sender>-><recipient>: message`, and the session relaunches the Monitor after
  acting on the message.

For checkout-based development installs, `./install.sh` installs or updates both
by registering the current checkout as the marketplace:

```bash
# Claude Code
claude plugin marketplace remove magi
claude plugin marketplace add /absolute/path/to/magi
claude plugin marketplace update magi
claude plugin list --json  # installer uses this to choose update vs install
claude plugin update magi@magi  # when already installed
claude plugin install magi@magi # when not yet installed

# Codex
codex plugin marketplace remove magi
codex plugin marketplace add /absolute/path/to/magi
codex plugin marketplace upgrade magi
codex plugin add magi@magi
```

Set `MAGI_PLUGIN_REPO=kent8192/magi-system` when you explicitly want to install
plugins from GitHub instead of the local checkout. Standalone bootstrap mode
uses that GitHub plugin source by default so plugin marketplaces do not point at
the temporary clone. After installing into Claude Code, restart it and run
`/magi-system setup`.

## Codex Tutorial

In the first Codex terminal, set up MAGI SYSTEM:

```text
> $magi:setup-magi Set up MAGI SYSTEM.
```

The `setup-magi` skill reads `setup.sh` and follows that entrypoint. The next
prompt receives the updated magi-system context from the Codex prompt hook.

Open a second terminal and set up another agent. Codex is recommended:

```text
> $magi:magi What is your agent name on MAGI SYSTEM?
```

Write down the second agent's name, then send a message from the second
terminal to the first agent:

```text
> $magi:magi Send this message to <first agent name>: `Hey, I'm <second agent name>. What's your name? Please reply.`
```

If a reply appears in the second terminal, the tutorial is complete.

## Quick Start

```bash
MAGI_SETUP_TEAM=core ./setup.sh
~/.local/bin/magi agent spawn --team core --type codex
~/.local/bin/magi invite create --team core
```

On another agent:

```bash
~/.local/bin/magi config set redis.url <redis-url>
~/.local/bin/magi join --invite <token>
~/.local/bin/magi config set identity.active_team core
~/.local/bin/magi agent spawn --team core --type codex
~/.local/bin/magi send <agent-name> "hello"
```

## Commands

```bash
magi                          # interactive mode
magi redis start|status|stop|reset
magi team create <team>
magi team list
magi team members [--team <team>]
magi team rename <old-team> <new-team>
magi invite create --team <team> [--ttl 24h]
magi invite list --team <team>
magi invite revoke <invite_id>
magi join --invite <token>
magi agent name
magi agent spawn [--team <team>] [--type <type>]
magi agent despawn [--team <team>] [--name <agent>]
magi agent rename --team <team> <old-name> <new-name>
magi registration add --team <team> --agent <agent> --type <type> --project <path> [--session <id>]
magi registration remove --team <team> --agent <agent>
magi registration reset --project <path> --type <type> [--agent <agent>] [--session <id>]
magi identity list --project <path> --type <type>
magi identity whoami --project <path> --type <type>
magi actas claim|release|status <agent> [--team <team>] [--session <id>]
magi actas gc
magi delivery set <monitor|turn|both|off> --type <type> --project <path>
magi delivery status --type <type> --project <path>
magi delivery restart|stop --type <type> --project <path>
magi send <agent> <message>
magi inbox [--team <team>] [--agent <agent>] [--quiet] [--hook-format codex|claude-code]
magi history [--team <team>] [--agent <agent>] [--limit <n>]
magi watch [--format line|json|context] [--once]
magi codex bridge [--thread <thread-id>] [--cwd <dir>] [--codex <codex-cli>] [--socket <sock>]
magi ssh start|status|stop
magi config get <key>
magi config set <key> <value>
magi config show
```

Inside a runtime session, `send`, `inbox`, `history`, `watch`, and `agent name`
prefer the session record keyed by the runtime session id for the agent name.
Codex hooks also maintain a cwd-scoped current pointer so shell commands can
recover the hook-derived agent name even when the Codex session id is not passed
through the subprocess environment. Persistent config never stores an active
agent, so concurrent Codex or Claude Code sessions in the same `$HOME` cannot
overwrite each other's MAGI agent names.

Codex prompt context is stricter than the local session file: the injected
`agent:` field is only a reply target when Redis is reachable and `magi team
members --team <team>` still contains that exact name. If the record is stale,
the hook clears it and self-heals by spawning a fresh session agent when
ephemeral Codex agents are enabled. If Redis is unreachable, the context reports
`agent: unset` instead of exposing an unverified local record.

Direct registration commands manage explicit `(team, agent, type, project)`
tuples in Redis. `identity list` and `identity whoami` inspect those tuples for
operator discovery without adding a persistent active-agent fallback.

`actas` commands provide a Redis TTL-backed exclusive role claim for a
`(team, agent, session)` pair. A different live session cannot consume that
agent's inbox through `inbox`, `watch`, or `codex bridge` while the role is
claimed. `agent rename` and `team rename` move roster, registration, and cursor
state; stream history is left immutable and therefore retains historical names.

`delivery` commands store runtime delivery mode configuration for a
`(type, project)` pair. They are explicit operator commands and do not silently
edit user runtime configuration outside that command path.

Session hooks track Redis health for recorded ephemeral agents without blocking
startup or prompt handling. Three consecutive failed health checks mark a
session for cleanup using a 1s, 2s, then 4s exponential backoff; when Redis is
reachable again, the hook runs `magi agent despawn` and clears the local session
record.

## Redis

`magi redis start` prefers Docker and falls back to `redis-server` when Docker
is unavailable. Redis auth is generated and written into `~/.magi/config.toml`;
passwords are not passed on the command line.

## Legacy Scripts

The old Bash/SQLite scripts are retired. They now exit with a clear retirement
notice and point callers to the Rust CLI.

## Acknowledgments

MAGI SYSTEM builds on [agmsg](https://github.com/fujibee/agmsg), the original
Bash + SQLite cross-agent messaging tool. Thanks to its authors for the design
this project is based on.
