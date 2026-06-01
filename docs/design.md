# magi Design

## Architecture

- CLI: Rust, `clap`, Tokio
- State directory: `~/.magi`
- Install locations: `~/.agents/skills/magi/bin/magi`, `~/.local/bin/magi`
- Codex plugin surface: `.codex-plugin/plugin.json` with `skills/magi/`
  mirroring the installed Codex skill instructions. The marketplace package at
  `plugins/magi/` mirrors those files at the package root because Codex
  marketplace sources point at a plugin directory and installed hooks/skills are
  resolved from that plugin root. A `.codex-plugin/` copy remains in the package
  for tooling that inspects the conventional Codex plugin manifest location.
- Claude Code plugin surface: `integrations/magi-agent-plugin/` (the event-driven
  `magi-agent` bridge) with its own `.claude-plugin/` manifests.
- Plugin distribution: the repository-root `.claude-plugin/marketplace.json`
  exposes the Claude Code `magi` marketplace, while
  `.agents/plugins/marketplace.json` exposes the Codex `magi` marketplace and
  points at `plugins/magi`. `install.sh` and `uninstall.sh` register and remove
  these through the `claude plugin` and `codex plugin` CLIs on a best-effort
  basis (skipped when the CLI is absent). Re-running `install.sh` refreshes the
  marketplace, updates an already-installed Claude Code plugin or installs it
  when absent, and re-adds the Codex plugin from the current marketplace
  snapshot.
- Redis lifecycle: Docker first, `redis-server` fallback
- Durable messaging: Redis Streams
- Wakeups: Redis Pub/Sub
- Inbox tracking: one Redis cursor per `(team, agent)`

## Redis Key Model

All keys use the `magi:` prefix. Team and agent segments are percent-encoded so
IDs containing separators cannot collide with normal key shapes.

Important keys:

- `magi:teams`
- `magi:team:<team>`
- `magi:team:<team>:agents`
- `magi:team:<team>:agent_seq`
- `magi:agent:<team>:<agent>`
- `magi:stream:<team>`
- `magi:cursor:<team>:<agent>`
- `magi:pubsub:<team>`
- `magi:invite:<invite_id>`
- `magi:invite_token:<token_hash>`

## Messages

Messages are appended to `magi:stream:<team>` with fields:

- `from`
- `to`
- `body`
- `created_at`

`magi inbox` reads from the stored cursor, prints messages addressed to the
active agent, and advances the cursor to the last scanned stream entry.

`magi watch` subscribes to `magi:pubsub:<team>` and also polls periodically, so
missed Pub/Sub wakeups do not lose durable Stream messages. `magi watch --once`
uses the same delivery path but exits after the first non-empty batch; with
`--format context` it prints `<sender>-><recipient>: message` for direct agent
context injection.

## Invites

Invite tokens are generated randomly. Redis stores only a SHA-256 token hash and
a lookup key with TTL. Joining is guarded by a Lua script so revoked, expired,
or exhausted invites cannot race through concurrent joins.

## Ephemeral Session Agents

`magi agent spawn` registers a short-lived, uniquely named agent into the active
team; `magi agent despawn` removes it. They exist so a CLI session can be treated
as a disposable agent that joins on start and leaves on end.

Names are `<adjective>-<magi>`:

- The `<magi>` suffix cycles deterministically through the three MAGI units —
  `melchior`, `balthasar`, `casper` — selected by `(<seq> - 1) % 3`, where
  `<seq>` is an atomic `INCR` of `magi:team:<team>:agent_seq`. The counter is
  monotonic and never decremented, so the cycle keeps advancing regardless of
  despawns rather than depending on the current member count.
- The `<adjective>` is drawn at random from the `petname` word list using the
  project's own `rand`, so a fresh agent reads as e.g. `quiet-melchior`.

Uniqueness is enforced by claiming each candidate with an atomic `SADD` to
`magi:team:<team>:agents`; on collision the next adjective is tried, and a final
`<adjective>-<magi>-<seq>` fallback cannot collide because `<seq>` is unique.

The Claude Code `magi-agent` plugin and the Codex `magi` plugin both drive this
lifecycle from session hooks. SessionStart spawns an agent (recording
`<name, team>` keyed by the runtime session id under the runtime state dir) and
SessionEnd despawns it. Claude Code uses `MAGI_AGENT_EPHEMERAL=0` to opt out;
Codex uses `MAGI_CODEX_EPHEMERAL=0`.

Interactive messaging commands resolve identity in two layers. When a runtime
session id is available (`MAGI_SESSION_ID`, `CODEX_THREAD_ID`,
`CODEX_SESSION_ID`, or `CLAUDE_SESSION_ID`), `send`, `inbox`, `history`,
`watch`, and `agent name` first read the matching session record under the Codex
or Claude Code state directory. Codex hooks also write a cwd-scoped current
pointer from the hook-derived context, which lets shell commands recover the
same agent name when the Codex session id is not inherited by the subprocess.
If no session record or hook current pointer exists, there is no agent-name
fallback; commands that need an agent fail instead of reusing another session's
name. The team can still fall back to `identity.active_team`.

The Codex plugin also injects a compact UserPromptSubmit context block before
each prompt with the resolved session id, active magi agent, active team, Redis
state, and session record status. If Codex was updated after a session started
and SessionStart did not create a record, UserPromptSubmit performs the same
spawn-and-record step before injecting context.

The Claude Code plugin uses the runtime's Monitor primitive when the autonomous
bridge is not running. SessionStart directs Claude Code to launch
`hooks/magi-monitor-once.sh <session-id>`, which blocks in
`magi watch --once --format context`. When Redis publishes a wakeup for this
agent, the Monitor process exits with `<sender>-><recipient>: message`; Claude
Code handles that completed background output and relaunches the same Monitor
command for the next message. If `/magi-system start` is running, the Monitor
directive is skipped so the SDK bridge remains the only inbox consumer.

## Plugin Parity (Codex vs Claude Code)

The messaging command surface is intentionally mirrored across the Codex `magi`
skill (`.codex-plugin/skills/magi/SKILL.md`) and the Claude Code `magi-messaging`
skill, both referencing the `magi` binary on `PATH`. Runtime-specific differences
are deliberate and **not** bugs to be "fixed":

- **Skill frontmatter** differs because the runtimes require different schemas: a
  Codex skill needs a `name:` field; a Claude Code slash command does not.
- **The auto-reply bridge** (`/magi-system`, the `magi-agent` skill) is a
  **Claude Code only** feature because it is built on the Claude Agent SDK.
  Claude Code also has a Monitor-based live-delivery path for foreground
  sessions. Codex has no Monitor tool, but it does have native plugin hooks for
  the ephemeral session-agent lifecycle and per-prompt magi-system context.

## SSH

`magi ssh start` creates an SSH local port-forward from the configured
`ssh.local_port` to `ssh.remote_host:ssh.remote_port` via `ssh.host`, and stores
the process id under `~/.magi/run`.

## Retired Bash Scripts

The former Bash/SQLite scripts remain only as compatibility stubs. Each exits
with code `2` and directs callers to the Rust CLI.
