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
  `magi` plugin with the `magi-agent` bridge) with its own `.claude-plugin/`
  manifests.
- Plugin distribution: the repository-root `.claude-plugin/marketplace.json`
  exposes the Claude Code `magi` marketplace, while
  `.agents/plugins/marketplace.json` exposes the Codex `magi` marketplace and
  points at `plugins/magi`. `install.sh` and `uninstall.sh` register and remove
  these through the `claude plugin` and `codex plugin` CLIs on a best-effort
  basis (skipped when the CLI is absent). Re-running `install.sh` refreshes the
  marketplace, updates an already-installed Claude Code plugin or installs it
  when absent, and re-adds the Codex plugin from the current marketplace
  snapshot. When `install.sh` is run outside a checkout, it bootstraps from
  `MAGI_BOOTSTRAP_REPO_URL` (defaulting to
  `https://github.com/kent8192/magi-system.git`) and delegates to that checkout,
  with `MAGI_PLUGIN_REPO` defaulting to the durable `kent8192/magi-system`
  plugin source instead of the temporary clone path.
- Setup entrypoint: `setup.sh` uses only the `magi` CLI to start managed Redis,
  create the selected setup team when needed, and set
  `identity.active_team`. Codex exposes a `setup-magi` skill whose instructions
  read and then run this entrypoint for MAGI SYSTEM setup prompts.
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

The Claude Code `magi` plugin and the Codex `magi` plugin both drive this
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

Operators can manage explicit project/type registrations with
`magi registration add/remove/reset`. Registrations keep the existing
`type:project` set value for compatibility, while optional session ownership is
stored in an adjacent Redis hash. `magi identity list` and
`magi identity whoami` inspect those registrations to report exact matches,
multiple matches, same-type suggestions in other projects, or a not-joined
state without restoring persistent active-agent config.

`magi actas claim` writes a TTL-backed Redis claim for a `(team, agent,
session)` pair. Inbox consumers (`inbox`, `watch`, and the Codex bridge) check
that claim before advancing a cursor. If another live session owns the claim,
the consumer fails before reading messages. `actas gc` is intentionally a
health command because stale claims expire via Redis TTL.

The Codex plugin also injects a compact UserPromptSubmit context block before
each prompt with the resolved session id, active magi agent, active team, Redis
state, and session record status. If Codex was updated after a session started
and SessionStart did not create a record, UserPromptSubmit performs the same
spawn-and-record step before injecting context.
Setup prompts are handled by the `setup-magi` skill, which reads `setup.sh` and
uses that script as the setup entrypoint instead of relying on prompt-string
matching inside hooks.

The Claude Code plugin uses the runtime's Monitor primitive to keep at least
one foreground inbox waiter available for each session. SessionStart,
UserPromptSubmit, PostToolUse, and Stop hooks check the session-scoped Monitor
pid sidecar; when no live Monitor is recorded, they direct Claude Code to launch
`hooks/magi-monitor-once.sh <session-id>`, which blocks in
`magi watch --once --format context`. When Redis publishes a wakeup for this
agent, the Monitor process exits with `<sender>-><recipient>: message`; Claude
Code handles that completed background output and relaunches the same Monitor
command for the next message.

Stop hooks additionally inspect Claude Code's `background_tasks` registry. If no
task with `type: "monitor"` is present for an active session, the hook returns a
blocking decision with the same Monitor launch directive so shutdown does not
complete before at least one Monitor is waiting.

Hooks also keep a small `<session>.health` sidecar next to each recorded
ephemeral agent. Failed Redis health checks are counted without sleeping in the
hook path: the next due check is scheduled with a 1s, 2s, then 4s backoff. After
three due consecutive failures the sidecar marks cleanup pending; the next hook
run that can reach Redis removes the agent through `magi agent despawn` and
clears the session record.

For live Codex delivery, SessionStart first ensures the managed Codex app-server
daemon is running, then launches `magi codex bridge --thread <session-id>` as a
session-scoped background process, and SessionEnd stops it. Prompt hooks perform
the same daemon check before restarting a missing bridge. The bridge subscribes
to the team Pub/Sub channel, drains unread inbox messages for the session agent,
formats each delivery as `<sender>-><recipient>: message`, connects directly to
the Codex Unix control socket with WebSocket-over-UDS, initializes the
app-server JSON-RPC session, and sends the message over that WebSocket
transport. The bridge first persists the message with `thread/inject_items`;
only after that injection succeeds does it best-effort start a Codex turn with
`turn/start` so the agent can act immediately.
`MAGI_CODEX_APP_SERVER_BRIDGE=0` disables this background bridge,
`MAGI_CODEX_CLI` overrides the Codex executable, and
`MAGI_CODEX_APP_SERVER_SOCKET` overrides the Unix control socket path.
`MAGI_CODEX_APP_SERVER_DAEMON=0` disables managed daemon autostart. The bridge
records a status sidecar under the Codex hook state directory so prompt hooks
can distinguish `starting`, `running`, `delivering`, `injecting`, `injected`,
`turn_started`, `retrying`, `unsupported`, `stopped`, and `disabled`. `running`
means the bridge process is alive and has no known delivery failure. `delivering`
means the bridge is actively draining unread messages, `injecting` means it is
persisting a message into the Codex thread, `injected` means durable injection
succeeded but the follow-up turn did not start, and `turn_started` means both
injection and the follow-up turn succeeded. If the Codex app-server control
socket is missing, the status becomes `unsupported` and the inbox cursor is not
advanced;
`stdio://` app-server processes cannot be reached by this external bridge.
Devcontainer use follows the same model: immediate injection requires the
container to see the host MAGI config/state, Codex hook state, and app-server
control socket. If any of those runtime surfaces are not reachable, the bridge
uses the same `unsupported` status rather than acknowledging delivery.
Other delivery failures remain `retrying` with the last error until a later
delivery succeeds.
Delivery failures are retried without acknowledging the failed message: when a
batch partially succeeds, the cursor advances only through the successfully
injected messages.

`magi delivery set/status/restart/stop` stores delivery mode metadata for a
`(type, project)` pair in Redis. These commands are the explicit operator path
for delivery-mode state; runtime hooks do not infer or mutate user config
silently outside that path.

Agent and team rename commands move roster, profile, registration, cursor, and
team metadata keys. Stream history remains immutable, so historical messages
keep the names that were recorded when they were sent.

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
  sessions. Codex uses a Codex app-server bridge instead: it does not run the
  Claude SDK or a Monitor tool, but it can turn incoming magi messages into
  Codex app-server turns for the current thread.

## SSH

`magi ssh start` creates an SSH local port-forward from the configured
`ssh.local_port` to `ssh.remote_host:ssh.remote_port` via `ssh.host`, and stores
the process id under `~/.magi/run`.

## Retired Bash Scripts

The former Bash/SQLite scripts remain only as compatibility stubs. Each exits
with code `2` and directs callers to the Rust CLI.
