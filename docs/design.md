# magi Design

## Architecture

- CLI: Rust, `clap`, Tokio
- State directory: `~/.magi`
- Install locations: `~/.agents/skills/magi/bin/magi`, `~/.local/bin/magi`
- Codex plugin surface: `.codex-plugin/plugin.json` with `skills/magi/`
  mirroring the installed Codex skill instructions.
- Claude Code plugin surface: `integrations/magi-agent-plugin/` (the event-driven
  `magi-agent` bridge) with its own `.claude-plugin/` manifests.
- Plugin distribution: the repository-root `.claude-plugin/marketplace.json`
  exposes the `magi-dev` marketplace listing both plugins (`magi` for Codex,
  `magi-agent` for Claude Code). `install.sh` and `uninstall.sh` register and
  remove these through the `claude plugin` and `codex plugin` CLIs on a
  best-effort basis (skipped when the CLI is absent).
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
missed Pub/Sub wakeups do not lose durable Stream messages.

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
  `melchior`, `balthasar`, `caspar` — selected by `(<seq> - 1) % 3`, where
  `<seq>` is an atomic `INCR` of `magi:team:<team>:agent_seq`. The counter is
  monotonic and never decremented, so the cycle keeps advancing regardless of
  despawns rather than depending on the current member count.
- The `<adjective>` is drawn at random from the `petname` word list using the
  project's own `rand`, so a fresh agent reads as e.g. `quiet-melchior`.

Uniqueness is enforced by claiming each candidate with an atomic `SADD` to
`magi:team:<team>:agents`; on collision the next adjective is tried, and a final
`<adjective>-<magi>-<seq>` fallback cannot collide because `<seq>` is unique.

The Claude Code `magi-agent` plugin drives this lifecycle from session hooks: the
SessionStart hook spawns an agent (recording `<name, team, prev-agent>` keyed by
the Claude session id under the daemon state dir) and the SessionEnd hook
despawns it and restores the previously active identity. Spawning is idempotent
per session id and opt-out via `MAGI_AGENT_EPHEMERAL=0`.

## Plugin Parity (Codex vs Claude Code)

The messaging command surface is intentionally mirrored across the Codex `magi`
skill (`.codex-plugin/skills/magi/SKILL.md`) and the Claude Code `magi-messaging`
skill, both referencing the `magi` binary on `PATH`. Two differences are
deliberate and **not** bugs to be "fixed":

- **Skill frontmatter** differs because the runtimes require different schemas: a
  Codex skill needs a `name:` field; a Claude Code slash command does not.
- **The auto-reply bridge** (`/magi-system`, the `magi-agent` skill) and the
  ephemeral session lifecycle are **Claude Code only**, because they are built on
  the Claude Agent SDK and Claude Code session hooks. Codex has no equivalent, so
  it exposes the `magi` CLI manually instead.

## SSH

`magi ssh start` creates an SSH local port-forward from the configured
`ssh.local_port` to `ssh.remote_host:ssh.remote_port` via `ssh.host`, and stores
the process id under `~/.magi/run`.

## Retired Bash Scripts

The former Bash/SQLite scripts remain only as compatibility stubs. Each exits
with code `2` and directs callers to the Rust CLI.
