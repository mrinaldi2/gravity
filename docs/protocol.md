# hermesd control plane protocol (v2)

Transport: WebSocket at `ws://<host>:49777/ws`. All frames are JSON text frames.
Every client→server message has a `req_id` (string, client-chosen, unique per connection).
Every server reply carries the same `req_id`. Server pushes (unsolicited) have no `req_id`.

## Handshake

Client sends first, immediately after connect:

```json
{ "type": "hello", "req_id": "1", "protocol_version": 2, "token": "<client token>", "client": "desktop/0.1.0" }
```

Server replies:

```json
{ "type": "hello_ok", "req_id": "1", "protocol_version": 2, "server_version": "0.1.0",
  "capabilities": ["terminal_attach", "search", "routines", "devices", "config", "decisions"],
  "grants": ["read", "control", "approve"], "device_id": null, "daemon_id": "…" }
```

`daemon_id` is the daemon's stable id, the one its peers learn (`Peer.daemon_id`).
A client connected to two daemons uses it to tell which peer row is which
daemon. `capabilities` includes `linked_projects` when the daemon supports
linked projects (`list_peer_projects`, `link_project`, `unlink_project`,
`create_bot` with `peer_id`, and `links` on projects), `bot_browser` when each bot
has a browser of its own the app can watch (`watch_browser`, `list_browser_activity`,
`set_bot_user_chrome`, `user_chrome` on bots), and `agent_conversations` when it serves
the conversations between a project's bots (`list_agent_conversations`,
`list_agent_conversation`), and `peer_terminal` when a linked bot's terminal can be
attached to, typed into and resized here, relayed from its machine, and
`peer_browser` when a linked bot's browser can be watched here the same way
(`watch_browser`, `list_browser_activity` on the stand-in), `browser_input` when
the owner can click and type into a watched bot browser (`browser_input`), and `bot_commands` when
`list_bot_commands` lists what each bot is running, and `restart_bot` when bots
can be restarted or cleared (`restart_bot`, `clear_bot_session`), and `workers`
when bots can spawn temporary workers (`temporary` on bots, `repo` on projects,
`set_project_repo`, `list_workers`, `cancel_worker`, the `workers_updated` push;
see [workers](workers.md)), and `permission_profiles` when projects carry a permission profile and bots their extras (`set_project_permission_profile`, `set_bot_permission_extras`).

`hello_ok.contracts` maps each typed surface to its contract version, e.g.
`{"board": 1}`. Those surfaces are defined once in protobuf under
`proto/hermes/<surface>/v1/` (ADR-001): Rust is generated at build time by
`crates/bus/build.rs`, TypeScript by `pnpm --dir apps/desktop proto:gen` into
`apps/desktop/src/protocol/gen/`, and Swift in the iOS repo from vendored copies.
Golden proto-JSON fixtures live in `crates/bus/fixtures/<surface>/`. A client may
send its own `"contracts"` map in `hello`. Within a version, changes follow
protobuf's compatibility rules; when `buf breaking` fails, the version is bumped.

`hello_ok.encodings` lists the binary encodings the daemon accepts, today
`["proto"]`. Text frames stay this JSON protocol. A binary frame carries one
`hermes.wire.v1.Envelope` (`req_id` plus a `body` oneof per typed surface) and
is answered with an `Envelope` under the same `req_id`; pushes carry `req_id` 0.
A request refused before its service answers (no grant, unknown item or
project, bad arguments, an envelope a client may not send) is answered with an
`Error` body: `forbidden`, `not_found`, `no_board`, `invalid_request` or
`internal`.

The board (`proto/hermes/board/v1/requests.proto`, H-020 §1.5) is the first
typed surface:

| Request | Grant | Response |
|---|---|---|
| `board_get {project_id}` | read | `board`: settings (with `home_daemon_id`), columns, cards, roles, `seq`. A project's first `board_get` enables its board, which needs control; with read only it is `no_board`. |
| `board_watch {project_id}` / `board_unwatch` | read | `board` (as `board_get`) / `unwatched` |
| `item_get {id}` | read | `item`: the item, links, comments, the first 100 history events and `history_next` |
| `item_history {id, after?, limit}` | read | `history`: events oldest first, `next` cursor |
| `item_query {project_id, text?, column_keys, assignee?, types, priorities, platforms, blocked?}` | read | `items`: matching cards |
| `item_move_check {id}` | read | `move_check`: every other column with its unmet guards |
| `item_move {id, to, expected_version, reason?, override_reason?}` | control | `moved`: `done` (the item), `refused` (unmet guards) or `conflict` (the current item) |

`board_watch` answers with the snapshot the pushes continue from, and no push
for that project is sent before it. Each committed board change is then pushed
to the connections watching its project as `board_event {project_id, seq, kind,
item_id, card?, from_column?}`, where `seq` rises by one per change in that
project. A client applies pushes with `seq` above its snapshot's and refetches
`board_get` when a push's `seq` is not the last one plus one (a lower number
means the daemon restarted) or its kind is `columns_changed`,
`settings_changed` or `resync` (sent when the connection fell behind).

`contracts: {surface: N}` is the **highest** version each side speaks, and it
stays an integer forever. A side that still serves older versions adds
`contracts_min: {surface: M}` (absent means M = N). The effective version per
surface is `min(client N, daemon N)`, and it must be at least both sides' M.
If no version fits, that surface is unavailable (its UI is hidden with "update
the app" or "update Hermes"); the connection itself is not refused. Only
`protocol_version` refuses a connection.

or `{ "type": "error", "req_id": "1", "code": "auth_failed" | "unsupported_version", "message": "..." }`
followed by close.

A client may add `"features": ["permission_cards"]` to `hello`: it shows bots'
permission prompts and can answer them (`answer_permission`). The daemon only holds
a Claude Code prompt for the app while at least one such client with the `control`
grant is connected; otherwise the prompt stays in the bot's terminal.

Two credential kinds are accepted as `token`:

- **Owner token** (`~/.thehermes/secrets/client.token`) — full grants, same machine.
- **Device token** — issued via `create_device`, carries scoped `grants`
  (any of `read`, `control`, `approve`) and can be revoked. A revoked device
  cannot reconnect.

Requests are gated by grants: `list_*`, `attach`, `detach`, `search`, and
`diagnostics` need `read`; everything else needs `control`. Violations get
`{ "code": "forbidden" }`.

Browser-context connections are additionally subject to Origin validation:
requests with an Origin header must match localhost/tauri defaults or the
daemon's `allowed_origins` config, otherwise the upgrade is rejected with 403.

## Keepalive

The server sends a WebSocket Ping every ~20 s and closes a connection that has
sent nothing at all for ~60 s (any frame counts: a request, a Ping, a Pong). Closing
it ends everything the connection was doing, as a normal close does: terminal
attachments, the browser watch, pushes. A client that pings on its own, or simply
answers the server's pings (WebSocket libraries do), stays connected. The server
answers a client's Ping with a Pong promptly; at most one `browser_frame` is ever
ahead of it.

## Errors

`{ "type": "error", "req_id": "...", "code": "<snake_case>", "message": "human text" }`
Codes: `auth_failed`, `unsupported_version`, `not_found`, `invalid_request`,
`runtime_unavailable`, `conflict`, `forbidden`, `internal`.

## Client → server requests

| type | fields | reply |
|---|---|---|
| `list_projects` | – | `projects` |
| `create_project` | `name` | `project` |
| `update_project` | `project_id, name` | `project` |
| `list_workers` | `project_id` | `workers`: `{ project_id, workers: [Worker], running_here, max_workers_here }`, queued and running oldest first, then recently finished. Requires `read` |
| `cancel_worker` | `worker_id, reason?` | `worker`: a queued spawn is dropped; a running worker is told to stop, in the owner's name, and retires. `conflict` once it has finished |
| `set_project_repo` | `project_id, url \| null, branch?` (default `main`) | `project`: the shared git repository workers check out and push to; `url: null` clears it. Bots' prompts are rewritten for their next start |
| `set_project_permission_profile` | `project_id, profile` (`standard` \| `trusted` \| `full`) | `project` (`permission_profile`). **approve** grant. What the project's bots may do without asking (H-031); its local bots restart to pick it up |
| `set_bot_permission_extras` | `bot_id, extras: [publish \| daemon_restart \| app_restart \| install \| release_main]` | `bot` (`permission_extras`). **approve** grant. Replaces the bot's extras, which apply in Trusted and Full (`release_main`, pushing and merging to `main`, in every profile); the bot restarts |
| `delete_project` | `project_id` | `ok` — archives the project and every bot in it; see Semantics |
| `list_bots` | `project_id?` | `bots` |
| `list_bot_activity` | `project_id?` | `bot_activity` — one preview line per bot; see Semantics |
| `create_bot` | `project_id, name?, description?, instructions?, avatar?, runtime?, peer_id?` | `bot` (starts running immediately). With `peer_id`, the peer creates the bot in the project linked with this one (`runtime` defaults to the peer's `default_bot_runtime`), and the reply is its stand-in here, with `peer` set; `not_linked` when the project is not linked through that peer, `unavailable` when it is offline |
| `set_bot_runtime` | `bot_id, runtime` | `bot` (requires `control`; restarts when changed) |
| `restart_bot` | `bot_id` | `ok`: the bot's session restarts and picks its conversation back up, and is told what it left unfinished (see Semantics). On a linked bot, restarted on its machine. Requires `control` |
| `clear_bot_session` | `bot_id` | `ok`: the bot restarts with a fresh conversation, without the old one's history (which leaves the chat); its workspace, memory files and tasks are kept, and it is told what it was working on. On a linked bot, done on its machine. Requires `control` |
| `set_bot_user_chrome` | `bot_id, enabled` | `bot`: whether the bot may also drive the owner's own Chrome (Claude in Chrome). Off by default; restarts the session. See [bot-browser.md](bot-browser.md) |
| `update_bot` | `bot_id, name?, description?, instructions?, avatar?` | `bot`; on a stand-in in a linked project, forwarded to its machine |
| `delete_bot` | `bot_id, reason?` | `ok` — archives the bot; see Semantics. On a stand-in in a linked project, deletes the bot on its machine |
| `list_bot_revisions` | `bot_id, limit?` | `bot_revisions` |
| `revert_bot_revision` | `revision_id` | `bot` |
| `attach` | `bot_id, after_seq?` (number) | `attached` then `term` pushes |
| `detach` | `bot_id` | `ok` |
| `input` | `bot_id, data` (utf8 string, may contain control chars) | none (fire-and-forget; requires `control` grant) |
| `resize` | `bot_id, cols, rows, force?` | none (requires `control` grant) |
| `send_user_message` | `to_bot_id, body` | `message` |
| `list_messages` | `conversation_id, before_id?, limit?` | `messages` |
| `list_conversations` | `project_id?` | `conversations` |
| `list_routines` | `bot_id` | `routines` |
| `create_routine` | `bot_id, name, trigger, prompt, overlap_policy, max_duration_seconds?, max_attempts?` | `routine` |
| `set_routine_enabled` | `routine_id, enabled` | `routine` |
| `run_routine_now` | `routine_id` | `ok` |
| `cancel_routine_run` | `routine_run_id` | `routine_run` |
| `list_routine_runs` | `routine_id, limit?` | `routine_runs` |
| `emit_signal` | `project_id, name, payload?` | `signal` |
| `list_deliveries` | `bot_id?, state?` | `deliveries` |
| `retry_delivery` | `delivery_id` | `ok` |
| `search` | `query, project_id?` | `search_results` |
| `diagnostics` | – | `diagnostics` |
| `get_config` | – | `config` |
| `set_config` | `auto_compact_window` (number or `null`) | `config` |
| `list_devices` | – | `devices` |
| `create_device` | `name, capabilities` (array of `"read"`/`"control"`/`"approve"`) | `device` (includes `token`, shown once) |
| `revoke_device` | `device_id` | `device` |
| `list_peers` | – | `peers` |
| `create_peer_invite` | `name, url?` (defaults to the first non-loopback bind address) | `peer` plus `invite` (holds the link token, shown once) |
| `add_peer` | `name, invite` | `peer`; the daemon starts dialing it |
| `revoke_peer` | `peer_id` | `peer` |
| `list_peer_bots` | `peer_id` | `peer_bots` (`bots`: id, name, description, avatar, runtime, project) |
| `link_peer_bot` | `peer_id, remote_bot_id, project_id` | `bot` (a linked bot; `peer` is set on it) |
| `list_peer_projects` | `peer_id` | `peer_projects`: `peer_id`, `projects: [{ id, name, bot_count, linked_project_id? }]`; `linked_project_id` is the project here it is already linked with. `unavailable` when the peer is offline. Requires `read` |
| `link_project` | `project_id, peer_id, remote_project_id?, remote_name?` | `project` with its `links`. With `remote_project_id`, links that project on the peer; without, the peer creates one named like this project (or `remote_name`). Errors: `not_found`, `unavailable`, `conflict` (bot names clash, naming them, or either project is already linked through that peer). See [peer-bots.md](peer-bots.md#linked-projects) |
| `unlink_project` | `project_id, peer_id` | `project`; archives the stand-ins on both sides. `not_linked` when there is no such link |
| `list_chat` | `bot_id, before?` (a turn id), `limit?` (default 30, max 200) | `chat` (`turns` oldest first, `has_more`) |
| `get_chat_step` | `bot_id, item_id` | `chat_step` (`detail`: `input?, command?, output?, diff?, content?`) |
| `get_chat_image` | `bot_id, image_id` | `file` |
| `list_artifacts` | `project_id, limit?` (default: every file; max 500), `before?` | `artifacts` (newest first, then by path), `has_more`, `next_before`. Each artifact may carry `created_by: { bot_id?, name, avatar, machine?, via }`, who made the file: `wrote`/`edited` (the first bot whose file tools touched it, by its transcript), `command` (the first bot whose command named its path), `upload` (the owner, from the app; no `bot_id`), or `sent` (a bot on another machine, with a result). Absent when it cannot be told. Pass a page's `next_before` as `before` for the next page; it is `null` once `has_more` is false. The cursor is the last file's modified time and path, so a file edited between pages moves to the top rather than shifting the rest. Without `limit` every file comes back, as older clients expect. |
| `write_artifact` | `project_id, name, base64` (one chunk, up to ~512 KB), `upload_id?` (from the first chunk's reply), `last` | `upload` (`upload_id`, and `path` once the last chunk is in); `control` grant. Files land in the project's `artifacts/uploads/`, up to 16 MB |
| `list_tasks` | `bot_id, state?` (`open` \| `closed`), `limit?` (default 100) | `tasks`: newest first, only open or only closed (`done`, `cancelled`, `expired`) ones when `state` says which, so a client can load every open task and a page of closed ones; each with `state`, `role` (`assigned` \| `delegated`), `other` (`name`, `machine?`), `request` and `result?` as previews (`request_truncated`, `result_truncated` say when they were cut), `deadline_at?`, `closed_at?` |
| `get_task` | `bot_id, task_id` | `task`: the same shape with the whole request and result |
| `watch_browser` | `bot_id, tab_id?` | `ok`, then `browser_tabs` and `browser_frame` pushes to this connection, starting with the current tabs and screen. `tab_id` shows that tab; without it the view follows the tab the bot used last. One watch per connection: a new one replaces it. Every connection watching a bot shares one stream. Frames are newest-wins: a slow connection skips frames rather than queuing them, and replies and pushes never wait behind more than one frame. Requires `read` |
| `unwatch_browser` | – | `ok`; stops the stream |
| `browser_input` | `bot_id, tab_id, event` | Fire-and-forget, as `input` is: no reply, and an `error` (null `req_id`) only when the event cannot be delivered. The owner's mouse or keyboard on a tab this daemon is streaming (`watch_browser`), to sign a bot in or get it past a page. `event` is one of `{ kind: "mouse", action: down\|up\|move, x, y, button: left\|middle\|right\|none, clicks, modifiers }`, `{ kind: "wheel", x, y, dx, dy, modifiers }`, `{ kind: "key", action: down\|up, key, code, key_code, text?, modifiers }` (`text` is what a key going down types) or `{ kind: "text", text }` (a paste). Coordinates are the page's CSS pixels, the `width` and `height` of `browser_frame`; `modifiers` is Alt 1, Ctrl 2, Meta 4, Shift 8. For a linked bot it is relayed to its machine. Requires `control` |
| `list_browser_activity` | `bot_id, limit?` (default 100) | `browser_activity`: `activity`, newest first, each `{ turn_id, step_id, at, browser: own\|owners_chrome, title, subtitle?, status, trigger }`; `trigger` is what started the turn, as in the chat |
| `list_bot_commands` | `bot_id, limit?` (default 100) | `bot_commands`: `commands`, running first then newest, each `{ id, command, description?, background, status: running\|done\|failed\|stopped, started_at, ended_at?, exit_code?, task_id?, output? }`. Read from the bot's transcript: a background command runs until the runtime reports it finished or the bot stops it, and a running one's `output` is the live end of its output file. A command left running when its session ended is `stopped`. For a linked bot, read from its machine. Refetch on `chat_turns` for the bot, and every few seconds while a background command runs. Requires `read` |
| `list_agent_conversations` | `project_id` | `agent_conversations`: `conversations`, most recent first, each `{ bot_ids: [a, b], message_count, last_at, last }`, and `bots` (`id, name, avatar, machine?, deleted`) naming everyone in them |
| `list_agent_conversation` | `project_id, bot_ids: [a, b], before?, limit?` (default 50) | `agent_conversation`: `messages` between the two, oldest first, each `{ id, num, from_bot_id, to_bot_id, kind, body, ref_message_id?, task?: { id, state }, created_at }`, `has_more`, and `bots`. `before` is a message `num` |
| `list_permissions` | `bot_id?` | `permissions` (prompts waiting on the owner) |
| `answer_permission` | `request_id, decision` (`allow_once` \| `allow_session` \| `deny`), `reason?` | `permission`; `control` grant |
| `read_file` | `path` and `bot_id` (its directory and its project's artifacts) or `project_id` (artifacts only) | `file` (`text` or `base64`, capped at 16 MiB) |
| `list_decisions` | `project_id?, state?, tag?, bot_id?, query?, before?, limit?` | `decisions` |
| `get_decision` | `decision_id` | `decision` (with comments, tags, notifications) |
| `count_pending_decisions` | – | `pending_decisions` |
| `answer_decision` | `decision_id, ruling_option?, ruling_text, ruling_reason?` | `decision` (state `answered`) |
| `unanswer_decision` | `decision_id` | `decision` |
| `hold_decision` | `decision_id, until?, comment?` | `decision` (state `held`) |
| `resume_decision` | `decision_id` | `decision` (state `open`) |
| `confirm_decision` | `decision_id` | `decision` — a bot-recorded ruling, now owner-confirmed |
| `withdraw_decision` | `decision_id, reason` | `decision` |
| `reopen_decision` | `decision_id, title?, body?` | `decision` (the new one) |
| `comment_decision` | `decision_id, body` | `decision_comment` |
| `update_decision` | `decision_id` plus any of `title, body, options, recommendation, priority, deadline_at, ruling_option, ruling_text, ruling_reason, renotify?` | `decision` |
| `delete_decision` | `decision_id` | `ok` |
| `publish_decisions` | `items: [{decision_id, notify_bot_ids?, ruling_option?, ruling_text?, ruling_reason?}]` | `publish_result` |
| `list_releases` | `project_id` | `releases` (newest first), each with `can_rule` and `rule_on` |
| `get_release` | `release_id` | `release`, with `can_rule` and `rule_on` |
| `release_rule` | `release_id, verdicts: [{item_id, verdict: ship\|hold\|rework, note?}], expected_version` | `release`. Needs `approve`, on the board's home daemon; see [Releases](#releases-and-the-deploy-gate) |
| `release_hold` / `release_unhold` | `release_id, note?, remind_at?` (RFC 3339) / `release_id` | `release`. Needs `approve` |
| `release_pause` / `release_resume` | `release_id, reason` / `release_id` | `release` |
| `set_decision_tags` | `decision_id, tags: [name]` | `decision` |
| `list_tags` | – | `tags` |
| `upsert_tag` | `name, description?, color?` | `tag` |
| `retire_tag` | `name, into?` | `tag` |
| `rename_tag` | `name, to` | `tag` |
| `delete_tag` | `name` | `ok` |
| `set_project_lead` | `project_id, bot_id \| null` | `project` |

When `create_bot` omits `name` — or sends it as `null` — the daemon allocates the first
available placeholder in the project: `New Bot`, `New Bot 2`, and so on, retrying if another
client claims the same one first. Explicitly named bot creation is unchanged.

Daemons advertising `bot_runtime` include `runtime: "claude_code" | "codex_cli"`
in bot objects. Older daemons omit it and use Claude Code. `create_bot` defaults
to the configured `default_bot_runtime`; bot-authored children inherit their
creator's runtime. `set_bot_runtime` preserves the workspace and provider histories,
interrupts the current turn, and restarts through the supervisor. Setting the
current value is a no-op. Existing bots migrate to `claude_code`.

`trigger` is `{ "kind": "cron", "expr": "0 0 9 * * MON", "tz": "Europe/Warsaw" }`,
`{ "kind": "interval", "seconds": 3600 }`, or
`{ "kind": "signal", "name": "deploy.finished", "from_bot_id": "..." }`.
`from_bot_id` is optional; omitting it matches any bot in the project.
`overlap_policy`: `"skip" | "queue_one" | "queue_all" | "replace"`.
`max_duration_seconds` must be between 1 and 21600, and `max_attempts` between
1 and 5. Signal payloads are limited to 16 KiB of encoded JSON. A signal-fired
routine receives the signal metadata and payload in a `Signal context` block
appended to its prompt.

Version 2 replaces the former `event` trigger variant with `signal`. This is a
breaking wire-shape change; v1 clients must upgrade before connecting.

## Server replies (carry `req_id`)

- `ok`: `{}`
- `project` / `projects`, `bot` / `bots`, `message` / `messages`,
  `conversations`, `routine` / `routines`, `routine_run` / `routine_runs`, `signal`, `deliveries`, `search_results`,
  `diagnostics`: payload under a field of the same name. `diagnostics` carries
  `stale_build`: true when the daemon binary on disk is newer than the running
  process, i.e. it was rebuilt but not restarted and is still serving the old
  MCP tool list.
- `attached`: `{ "bot_id", "seq", "resumed" }` — `seq` is the last sequence
  number the replay covers and `term` pushes follow up to it. `resumed` says the client's
  `after_seq` was still contiguous with the server ring and its screen is intact; when it is
  false the client must clear its terminal. A replay the ring can no longer start at the first
  byte of the session starts at the first line break it still holds instead, so it never paints
  the half-written line eviction left behind. `force` on `resize` asks the
  daemon to nudge the pty size so the runtime repaints even when the dimensions match.
- `bot_activity`: `{ "activity": [{ "bot_id", "from", "text", "at" }] }` — `from` is empty
  when the bot itself spoke. Bots that have said nothing are omitted.
- `config`: the daemon configuration under `config` (shape below). Replied to both
  `get_config` and `set_config`; the latter echoes the state after the write.
  `port` is what the daemon serves and points bots at; `configured_port` is what
  `hermesd.toml` asked for. They differ when the configured port was occupied
  and a fallback was negotiated, which also moves the bus `/mcp` URL.

## Server pushes (no `req_id`)

- `term`: `{ "bot_id", "seq": <u64>, "data": "<utf8 terminal output>" }` — ordered per bot.
  One push may carry several consecutive pty reads merged together, up to 64 KB of output;
  `seq` is then the newest read included, and it is the cursor to resume from. A replay
  arrives the same way, so a full ring is a handful of pushes rather than one per read, and
  `attached.seq` still marks where it ends.
- `bot_state`: `{ "bot_id", "state", "reason", "at" }` — state ∈ `starting|ready|working|waiting_for_user|waiting_for_approval|rate_limited|auth_failed|crashed|stopping|stopped`.
- `message_new`: `{ "message": {...} }` — any new bus message visible to the user.
- `bot_updated`: `{ "bot": {...} }` — a bot was created, edited, or archived. Bots edit themselves unprompted, so clients must not cache identity across this push.
- `project_updated`: `{ "project": {...} }` — a project was created, renamed, archived, or linked or unlinked through a peer. As with `bot_updated`, archival is signalled by `deleted_at` being set rather than by a separate frame.
- `workers_updated`: `{ "project_id" }` — a project's worker queue changed: a spawn was queued, placed, waits for a new reason, or finished. Clients refetch `list_workers`.
- `activity_update`: `{ "activity": { "bot_id", "from", "text", "at" } }` — a bot's preview
  line changed. Sent when a finished turn becomes readable in the transcript, which lags
  the `ready` state; see Semantics.
- `delivery_update`: `{ "delivery": {...} }`.
- `permission_request`: `{ "request": { "id", "bot_id", "tool", "summary", "input", "created_at", "expires_at", "origin"? } }`
  — a bot's tool waits on the owner. Unanswered by `expires_at` (config
  `permission_timeout_seconds`, default 600) it is denied.
- `permission_resolved`: `{ "request_id", "bot_id", "outcome" }` — outcome ∈
  `allowed_once|allowed_session|denied|expired|abandoned` (the hook went away first).
- `browser_tabs`: `{ "bot_id", "open", "tabs": [{ "id", "title", "url" }], "active", "following", "reason"? }`
  — sent to a connection watching that bot's browser (`watch_browser`) whenever its tabs
  change. `open` is false while the bot has no browser running.
- `browser_frame`: `{ "bot_id", "tab_id", "data", "width", "height" }` — the newest screen of
  the tab on show, a base64 JPEG. Only sent while watching. Newest-wins: at most one is
  being written to a connection at a time, a newer screen replaces one not yet sent, and
  other traffic goes first, so frames may be skipped on a slow link.
- `chat_turns`: `{ "bot_id", "turns": [...] }` — turns of a loaded chat that are new or
  changed, usually the open one. Merge by turn `id`. Only bots whose chat a client has
  listed are followed. The turn model is described in
  [the chat pane design](superpowers/specs/2026-09-16-chat-pane-design.md).
- `routine_run_update`: `{ "routine_run": {...} }`.
- `approval_pending`: `{ "bot_id", "detail" }` — bot is waiting on its native permission prompt.
- `notify`: `{ "level": "info|warn|error", "title", "body", "decision_id"? }` —
  `decision_id` is set when the notice is about a decision, so a client can open
  the record rather than leaving the owner to go looking behind a toast.
- `decision_update`: `{ "decision": {...} }` — raised, edited, answered, held,
  published or withdrawn. Carries the assembled view, never a bare row.
- `decision_deleted`: `{ "decision_id" }`.
- `decision_comment_new`: `{ "comment": {...} }`.

## Entity shapes (JSON)

```jsonc
Project  { "id", "name", "dir_name", "lead_bot_id"?, "links", "repo": {"url","branch"}|null,
           "deleted_at"?, "created_at" }
ProjectLink { "peer_id", "peer_name", "online", "remote_project_id",
           "remote_project_name", "linked_at" }
Bot      { "id", "project_id", "name", "description", "avatar", "instructions",
           "state", "state_reason", "unread_count", "workspace_path", "dir_name",
           "created_by_bot_id"?, "user_chrome", "temporary", "deleted_at"?, "created_at" }
BotRevision { "id", "bot_id", "changed_by", "field", "old_value", "new_value",
           "created_at" }
Conversation { "id", "project_id", "bot_id", "title" }
Message  { "id", "conversation_id", "sender": {"kind":"user"|"bot"|"routine", "bot_id?", "name"},
           "kind": "task"|"reply"|"done"|"note"|"chat", "body", "ref_message_id?", "created_at" }
Delivery { "id", "message_id", "bot_id", "state": "queued"|"leased"|"delivered"|"acknowledged"|"failed",
           "attempt_count", "next_attempt_at", "last_error", "created_at" }
Routine  { "id", "bot_id", "name", "trigger": {...}, "prompt", "overlap_policy",
           "enabled", "max_duration_seconds?", "max_attempts", "next_run_at", "created_at" }
Trigger  { "kind": "cron", "expr", "tz" } | { "kind": "interval", "seconds" }
         | { "kind": "signal", "name", "from_bot_id?" }
RoutineRun { "id", "routine_id", "scheduled_for", "state": "scheduled"|"running"|"succeeded"|"failed"|"skipped"|"cancelled",
             "source": "schedule"|"manual"|"signal", "attempt", "signal_id?", "message_id?",
             "delivery_id?", "started_at", "deadline_at?", "next_attempt_at?", "finished_at", "error" }
Signal   { "id", "name", "source": "bot"|"manual", "project_id", "from_bot_id?", "payload",
           "origin_chain", "hop_count", "emitted_at" }
Diagnostics { "daemon_version", "protocol_version", "db_healthy", "runtime": {"kind","available","version?"},
              "delivery_backlog", "active_bots", "uptime_seconds" }
Config   { "bind": ["127.0.0.1"], "port": 49777, "configured_port": 49777,
           "runtime": "pty"|"double", "auto_compact_window": 250000|null }
Worker   { "id", "project_id", "name", "state": "queued"|"running"|"done"|"cancelled"|"expired"|"failed",
           "queue_position"?, "machine"?, "parent_bot_id", "parent_name"?, "brief", "task_id"?,
           "note"?, "bot_id"?, "created_at", "started_at"?, "finished_at"? }
Device   { "id", "name", "capabilities": ["read"|"control"|"approve"], "created_at",
           "revoked_at?", "last_seen_at?" }
Decision { "id", "project_id", "kind": "question"|"decision", "title", "body",
           "options": [{"key","label","description?"}], "recommendation?",
           "raised_by": {"bot_id","name","avatar"}, "on_behalf_of_bot_id?",
           "origin_chain", "source_message_id?", "source_task_id?",
           "priority": "normal"|"urgent", "deadline_at?",
           "state": "open"|"answered"|"held"|"settled"|"withdrawn", "held_until?",
           "ruling": {"option?","text","reason?","answered_at","answered_by"}?,
           "published_at?", "supersedes_id?", "superseded_by_id?", "withdrawn_reason?",
           "tags": ["..."], "comment_count", "last_comment_at?",
           "comments": [...], "notifications": [...], "edited_at?", "created_at" }
DecisionComment { "id", "decision_id", "author_kind": "bot"|"user", "author_bot_id?",
           "author_name", "body", "created_at" }
Tag      { "id", "name", "description", "color", "created_by", "retired_at?",
           "uses": {"<project_id>": n}, "last_used_at?", "open_uses" }
PendingCounts { "by_project": {"<project_id>": n}, "total", "urgent", "due_soon" }
PublishResult { "decision_id", "notified": ["<bot name>"],
           "skipped": [{"bot","reason"}] }
```

`Project` gains `lead_bot_id?`; `Message` gains `decision_id?`. `links` is
always present (empty when the project is not linked); `project_updated` is
pushed whenever a project's links change. A bot standing in for one on a peer
carries `peer: { id, name, online }`. A bot with `temporary: true` is a worker:
created for one task and archived once that task closes.

Timestamps are RFC 3339 UTC strings. IDs are UUIDv4 strings.

## Semantics

- Bots are always-on: there is no `start_bot`/`stop_bot`. The daemon starts
  every live bot when it boots, starts a newly created bot straight away, and
  restarts crashes with backoff (2s doubling to 5 minutes), so `bot_state` is
  something clients observe, never something they drive. A bot only reaches
  `stopped` when the daemon stops it while archiving.
- Whenever a bot's session starts again (the daemon starting, a crash restart,
  a runtime or Chrome change, `restart_bot`, `clear_bot_session`), a bot that
  was cut off mid-turn, or still holds open tasks, is sent one `note` from the
  daemon: what started the interrupted turn, its open tasks with their
  `task_id`s, and to pick up where it left off (or keep waiting, if it was
  waiting on someone). After `clear_bot_session` the note says the
  conversation was cleared and points at `FACTS.md`. It is read from the
  transcript just before the session starts; a note still waiting to be
  delivered is not sent twice. `resume_after_restart = false` in
  `hermesd.toml` turns it off.
- Multiple clients may `attach` to the same bot; all receive `term` pushes. Any
  connection with the `control` grant may `input`/`resize` — there is no input
  lease. Bus deliveries are posted to the bot session's inbox socket
  (cross-session messaging), so they are read between tool calls or start a new
  turn when idle, and never interleave with terminal input.
- Reconnect: client re-`attach`es with `after_seq` = last seen seq; server replays buffered
  output after that cursor (in-memory ring buffer; if the cursor fell out of the buffer the
  server replays what it has and sets `attached.seq` accordingly — client should clear screen).
- `list_bot_activity` answers "what did this bot last say?", which the bus alone cannot:
  a bot's conversation with the user happens in its Claude Code terminal, a raw pty that
  never produces bus messages. The daemon reads the newest assistant turn out of Claude
  Code's own JSONL transcript (under `~/.claude/projects/<workspace-path-with-/-and-.-as-->`)
  and returns whichever of that and the bot's newest DM message is more recent. Use it for
  the initial snapshot only, and take live updates from `activity_update`.
- `activity_update` exists because Claude Code's `Stop` hook fires *before* the finished
  turn reaches the transcript — by roughly 250ms. A client that refetches on `bot_state`
  going `ready` therefore reads the *previous* turn and stays one reply behind. The daemon
  absorbs that race instead: on turn end it polls the transcript until it carries something
  at least as new as the turn, then pushes. A turn that only ran tools produces no text and
  so no push.

- `set_config` (advertised as the `config` capability, requires the `control`
  grant) writes only `auto_compact_window`: a number in 100000–1000000, or
  `null` for the model default. The value is persisted in the daemon's database
  — layered over `hermesd.toml`, which stays untouched — and takes effect the
  next time each bot's session starts. `bind`, `port` and `runtime` in the
  `config` reply are read-only: they describe how the daemon was launched and
  change only via `hermesd.toml` plus a restart.

## Bot identity on the bus (H-044)

Bots run as the owner's user, so any secret a bot holds can be read by every other bot. A bot is therefore known by its process, not by a token:

- **Endpoint:** the daemon listens on a local endpoint. On macOS and Linux it's `<home>/run/bus.sock` (its directory is 0700). On Windows it's the named pipe `\\.\pipe\thehermes-<user SID>-<home hash>-bus`, which only that user may open. The path carries no authority.
- **Session config:** each session's `mcp.json` (and Codex's `mcp_servers`) declares the bus as a stdio server, `hermesd bus-proxy --endpoint <endpoint>`. The session starts the proxy as its own child, and the proxy relays newline-delimited JSON-RPC both ways.
- **Who is calling:** the daemon asks the OS for the process on the other end, then walks its parents, at most eight levels and each parent no younger than its child, to a session root. The supervisor records a session root at every start: the terminal CLI, or the Codex app server.
  - The caller is that root's bot, served by the same dispatcher as `POST /mcp`.
  - A caller under no root (a terminal, a process that detached) gets JSON-RPC error `-32001 not a bot session` and is disconnected.
  - The root is checked again on every request, so a restarted session's old processes are cut off.
- **`[auth] bot_bearer`:** `accept` (the default, phase 1) still takes a bot's bearer token on `POST /mcp` and the hook endpoints; `refuse` (phase 2) answers 401. Every bearer use is recorded per bot, so a machine moves to `refuse` once no bot has used one for a day.
- **`[auth] bot_transport`:** `stdio` (the default); `http` writes the old HTTP entry back into each bot's config on its next start, as a rollback.

### The owner on the endpoint (T4)

The owner is known by their app's code identity or by an OK from them in the app, not by `client.token`, which any bot can read.

- **Pin:** `hermesd service install` run from inside the app writes `<home>/secrets/owner-app.json`. On macOS this holds the app bundle's designated requirement. On Windows it holds the folder the app is installed in. The daemon reads the pin again on every check.
- **Where to connect:** the daemon writes its endpoint address to `<home>/run/endpoint`.
- **Methods.** A client sends one JSON-RPC request on the endpoint, before any bot traffic, and the connection closes after the answer.
  - `hermes/owner_ticket`: the daemon checks the caller against the pin. On macOS that's the peer's audit token checked against the requirement. On Windows the peer's executable must be inside the pinned folder and must not be `hermesd`. A caller that passes gets `{"ticket": "…"}`.
  - `hermes/owner_request {"command": "…", "cwd": "…"}`: the daemon raises a permission card filed under bot id `terminal` (tool `Terminal command`) and waits for the owner's answer. If the owner allows it, the caller gets a ticket.
  - The card's `origin` (also its `input`) carries separate fields (UX-014): `command` (no pid), `pid`, `process` (the pid's executable name), `launched_from` (the nearest ancestor that is an app or terminal: Terminal, iTerm2, Code, claude…), `cwd` (read from the OS; the client's `cwd` only where the OS can't tell, i.e. Windows) and `bot` (the name of the bot whose workspace holds `cwd`). A field that can't be read is left out. The client composes every line and never shows `summary`.
  - The app answers these cards with `allow_once` ("Allow this command") or `deny` (no reason); it offers no "Allow for session" and no single-key shortcuts for a terminal command.
- **Tickets:** single use, valid for 60 s. A ticket is passed as the `token` in WS `hello` and grants the same capabilities as the client token.
- **Refusals:** error `-32002`.
  - A caller inside a bot session is always refused: "a bot can't act as the owner" for `owner_ticket`, "Commands that act as the owner can't run from a bot's session." for `owner_request`.
  - Other refusals: "not the owner's app"; for `owner_request`, which the CLI prints as is, "You denied this command in The Hermes. It did not run.", "No answer in The Hermes, so the command did not run." and "Open The Hermes on this computer to allow this command, then run it again." (no app is connected to show the card).
- **Clients:** the desktop app asks for a ticket on every connect. `hermesd` CLI owner commands ask the owner to allow them, printing "Asking for your OK in The Hermes app…". Both fall back to `client.token` only when the daemon has no endpoint (a daemon older than T4). The daemon accepts `client.token` until T6.

## Bot self-management

Advertised as the `bot_self_management` capability in `hello_ok`. Every change
in this section is additive — new request types, one new push, new fields on
`Bot` — and did not independently require a protocol-version change.

Bots manage their own identity and each other over the MCP bus
(`get_self`, `update_self`, `rename_self`, `create_bot`, `update_bot`,
`delete_bot`). Nothing is queued for approval — a bot's change is live the
moment the tool returns. Three things bound that:

- **`max_bots_per_project`** (default 12) is the only limit on creation of
  permanent bots; temporary workers have their own `max_workers_per_project`
  and queue (see [workers](workers.md)). Since
  a bot may delete only its own children, and archived bots free their slot,
  this caps the live population however deeply bots nest their teams. There is
  no spawn-depth or rate limit; neither would constrain anything the population
  cap does not.
- **Direct parentage only.** A bot may edit or delete bots it created, and
  nothing else. Authority is not transitive: if A created B and B created C,
  A cannot touch C. A bot cannot delete itself.
- **`bot_revision`** records every identity change with its author, and
  `revert_bot_revision` restores the previous value. This is what replaces an
  approval step: changes are not prevented, they are reversible.

Only `name` is required to create a bot, from either entry point. A bot
created without a description or instructions is given a placeholder charter
that tells it to ask its creator what it is for and to record the answer with
`update_self` — so a bare "create a bot called Steve" yields a running bot
rather than a question.

`avatar` is a validated short string — `icon:<name>`, naming one of the twenty
built-in bot icons the client bundles (`orbit`, `ember`, `moss`, `nova`,
`tide`, `quartz`, `volt`, `dusk`, `copper`, `frost`, `halo`, `glitch`, `slate`,
`bloom`, `echo`, `pixel`, `rune`, `cloud`, `comet`, `mint`), or `color:#rrggbb`,
or empty for a client-derived swatch. Paths, `data:` URIs and URLs are rejected,
so a bot names an icon rather than shipping bytes. A bot created without an
avatar is dealt one of the icons at random.

### A project's directory never moves

A project owns `~/.thehermes/projects/<dir_name>/`, and every bot's
`workspace_path` points inside it. `dir_name` is derived from the name at
creation and then frozen, so `update_project` changes only what the project is
called: nothing on disk moves and no running bot loses its workspace. The name
still appears in `project.json` and each bot's `bot.json`, and both are
rewritten on rename so the files never contradict the database.

### Deleting a project takes its bots with it

`delete_project` archives every bot in the project through the same path as
`delete_bot` — stopped, credential revoked, open tasks released — and then
archives the project row itself. Skipping that would leave live runtimes holding
valid credentials for a project the user believes is gone.

Like a bot, a project is archived rather than deleted (`bot.project_id` and
`conversation.project_id` are foreign keys into `project`) and its name is
tombstoned so it can be reused immediately. Its directory is kept; the
workspaces inside it are reclaimed by retention on the usual
`archived_bot_days` schedule.

### Deletion is archival

`delete_bot` does not remove the row: `message.sender_bot_id` and
`conversation.bot_id` are foreign keys into `bot`, so deleting it would orphan
every message the bot ever sent. Instead the daemon stops the runtime, revokes
the bot's token, cancels its open tasks and tells each requester, archives the
DM conversation, and sets `deleted_at`. The bot leaves `list_bots`, addressing
and the population cap; its history stays readable.

The name is freed immediately (the archived row is tombstoned to
`<name>#<id-prefix>`), so create → delete → create with the same name works.
The workspace is kept and reclaimed by retention after `archived_bot_days`,
because deleting a bot should never destroy work it produced.

## The decision registry

Advertised as the `decisions` capability. Additive: new request types, three
new pushes, one optional field on `notify`, and new columns on `Project` and
`Message`. No protocol-version change, for the reasons given under Bot
self-management.

A **decision** is a durable, project-scoped record raised by a bot that needs
the owner's ruling. Bots raise, read, comment, withdraw and tag over MCP; the
owner answers and publishes over this protocol. The owner never raises one —
they tell a bot, and the bot files it — which is why there is no
`raise_decision` request here.

### Who may do what

Reads (`list_decisions`, `get_decision`, `list_tags`,
`count_pending_decisions`) need the `read` grant.

Ruling on a decision — `answer_decision`, `unanswer_decision`, `hold_decision`,
`resume_decision`, `confirm_decision`, `reopen_decision`, `update_decision`,
`delete_decision`, `publish_decisions` — needs `approve`. Everything else,
including `comment_decision`, `withdraw_decision` and the tag requests, needs
`control`.

`approve` is separate from `control` because the registry is only worth
anything if authority is legible. A credential that can start bots and send
messages is not thereby the owner, and a ruling published from one would be
indistinguishable from one they typed. A phone granted `read` and `approve` can
rule without being able to touch a terminal; a CI credential with `control`
runs the fleet without being able to answer.

Bots cannot answer, hold, publish, edit or delete. A project's lead bot is told
about every decision raised there and is pre-selected when one is published,
but it cannot answer for the owner either. That is the point: the live projects
showed a lead relaying a ruling it did not have the authority to give, and the
other bots were right to refuse it.

### States

```
open ──answer──▶ answered ──publish──▶ settled ──reopen──▶ (new decision, supersedes the old)
  │   ◀─unanswer──┘
  ├──hold──▶ held ──resume──▶ open
  └──withdraw──▶ withdrawn
```

- `answered` is a draft the owner alone can see. It exists so they can work
  through a batch and publish once.
- `held` is the owner saying "later" out loud; the asking bot is told, and a
  `held_until` that passes brings it back on its own.
- `settled` is authority. Bots cannot change it; reopening creates a new
  decision that supersedes it, so a topic that returns is decided on current
  facts rather than edited into a different answer.
- Only `open` and `answered` count toward `count_pending_decisions`.

### Publishing

`publish_decisions` settles one or several decisions, each with its own notify
set. Omitting `notify_bot_ids` tells the asker plus whoever the ask was raised
on behalf of. A bot that was already told is not told twice; one that cannot be
reached — archived, or in another project — comes back under `skipped` with the
reason, so the owner is never left believing a bot was notified when it was
not. A stopped bot is still notified: the bus is durable, and it reads the
ruling when it starts.

The ruling reaches a bot as an ordinary `note` carrying `decision_id`, rendered
as its own envelope:

```
[decision 7f3a from USER · settled · re "Waive rule 3?" · tags spend, apple-ads] Let it fire.
Option: Start today. Reason: Rule 8 is waived by me knowingly.
Raised by auction on 12 Sep. Deadline 13 Sep.
Full record: get_decision 7f3a. This is the owner's ruling, not a relay. Do not re-raise it; if the facts change, raise a new decision that supersedes it.
```

Sender `USER`, authenticated by the daemon, in the owner's verbatim words. That
is the authority a peer's relay can never be.

### Rulings given at a bot's terminal

A bot can file one with `record_decision`. The record is born `settled` with
`answered_by` = `owner-via-bot:<bot id>`, so the registry says plainly that it
was relayed rather than typed. `confirm_decision` rewrites that to `owner` and
re-notifies. Until then, other bots read it for what it is: a relay with a
paper trail.

### Tags

One taxonomy shared by every project, so a ruling can be found across both.
Names are `[a-z0-9-]{1,32}`, lowercased on write. Retiring hides a tag from
pickers but keeps its links, and `retire_tag` with `into` merges. A bot may not
retire a tag carrying more than ten settled decisions; unfiling the owner's
history is their call, and the tool error says so. `rename_tag` keeps every
link, because decisions reference the tag id rather than its name. `delete_tag`
removes the tag from every decision that carried it, leaving the decisions
themselves untouched, and is the owner's alone — bots have neither request.

## Releases and the deploy gate

DevOps assembles a release package over MCP (`release_create`,
`release_attach_build`, `release_submit`). Submitting checks every item as the
Verify → Owner testing move would, moves them there, freezes the package (a
hash of its items, builds and test results) and raises a decision for the
owner. That decision is a release decision: `release.decision_id` names it.

Only `release_rule` settles a release decision. `answer_decision`,
`publish_decisions`, `hold_decision`, the other ruling requests and a bot's
`withdraw_decision` are refused on it, and a bot can't call `release_rule` at
all, so no relayed release ruling exists. The ruling checks the frozen hash
and the version the owner reviewed, then applies the verdicts:

| Verdicts | Release | Items |
|---|---|---|
| all `ship` | `approved` | → Deploying |
| some `ship` | `repackaging` | ship stay in Owner testing; hold → Ready; rework → Doing |
| all `hold` | `held`; the decision is held, not settled | stay |
| no `ship` | `rejected` | hold → Ready; rework → Doing |

`release_deploy` (DevOps) re-checks the gate every time: an approved package,
a settled ruling answered by `owner` or `device:*` and not relayed, an
unchanged hash, and only shipped items. It opens a task to the machine's
tester (the bot holding the `tester` role for that machine), who fetches the
verified builds with `install_release` and reports with `deploy_confirm`.
When every required machine reports `ok`, the items move to Done and the
release to `deployed`. A failure moves them back to Verify, sets
`partially_deployed` and opens a rollback task to DevOps; `release_rollback`
runs through the tester the same way.

**Lifecycle (H-020 §6).**

- **Successor:** after a mixed ruling (`repackaging`) or a failed deploy (`partially_deployed`), DevOps calls `release_create` with `from`. The predecessor's shipped items join straight from Owner testing, and the predecessor becomes `superseded` when the successor is **submitted**. The successor is a new build, so it gets a new ruling. A failed deploy clears its items' `release_id`.
- **Cancel:** `release_cancel {release_id, reason?}` (DevOps) removes a package that is still `assembling` or `built`; anything submitted or later is refused. Its items were never moved: a predecessor's shipped items stay in Owner testing for the predecessor, which can take a new successor, and items from Verify are free again. A `cancelled` event keeps who, why and what it held, and shows in the predecessor's `events`.
- **Testing:**
  - DevOps fills `changelog` and `how_to_test` (`[{item_id?, platform, steps}]`) with `release_update` while the package is assembling.
  - Each machine's tester records a result against one of the builds' sha256 with `release_test`. The build must be for that machine's platform: one whose `required_machines` lists it, or with none configured for it, the items' platforms (a build platform `desktop-mac` is a `desktop` build).
  - `release_submit` is refused until every required machine has passed the current builds. Required machines are the board's `required_machines` for the items' platforms, or with none configured, every machine with a tester; deploys finish on the same set. With neither, submit is refused: an empty set is never a pass.
- **Hold:** `release_hold` keeps the decision open and holds it (`remind_at` becomes its `held_until`). When the reminder comes due, the decision sweep resumes it and the package goes back to `awaiting_owner`. `release_unhold` does the same on request.
- **Pause:** `release_pause` (owner, or DevOps over MCP) pauses a rollout in progress. Deploys and installs are refused with the reason, and every tester holding an open deploy task gets a note. `release_resume` returns it to `deploying`.
- **Who may rule:** `can_rule` says whether this connection may rule (the approve grant, on the board's home). When it can't, `rule_on` names the home computer. `BoardSnapshot.can_rule` carries the same flag.
- **Serving builds (H-020 §6.6):** `release_publish {release_id, file, platform?, version?, bundle_id?}` (DevOps with the Publish extra; `hermesd release publish <release> <file>` calls it with the bot's token) copies a build into the served directory, `[releases] dir` (default `<home>/releases`), at `<release_id>/<platform>/<file>`.
  - The file must be an `.ipa`, `.zip`, `.dmg`, `.exe` or `.msi`, and must resolve, symlinks and all, inside `[releases] source_roots` (default: the `<repo>-rel-*` release worktrees in the trusted paths) or the project's artifacts. A hard-linked file is refused, and the file is hashed and copied from one open handle. Inside the served directory, symlinks are refused; copies are staged beside it, never in it.
  - The served directory must be a real folder of its own: a symlink, or a folder that is or contains the daemon home, its secrets or `bus.sqlite`, is refused at config load and at every publish. Only the daemon writes there: every bot's guard and settings deny writes to it (CE-010).
  - A published file is never replaced by a different one.
  - It attaches the build with its sha256, its `url` under `[releases] base_url`, and its `install_url`. For an `.ipa`, that is the `itms-services://` link to a generated `manifest.plist`; for other builds, it is the `url`.
  - `install_release` re-hashes every served build against the frozen sha256 and returns `verified` per build. On a mismatch, it refuses the install, pauses a rollout in progress and tells DevOps.

## The project dashboard

`dashboard_get {project_id}` (read) answers with `{type: "dashboard", dashboard}`, which holds what the dashboard's first four widgets show (H-018 §2.1, H-076):

- **`needs_you`:** one row per thing waiting on the owner, each with a `kind`:
  - `release`: the package, with `can_rule`. Its decision gets no row of its own.
  - `decision`: an open decision of the project, or a relayed ruling waiting for confirmation (`relayed: true`).
  - `p0`: an open P0 item.
  - `wip_override`: a move made over a WIP limit in the last seven days, with its note.
- **`board`:** null without a board. Otherwise:
  - `columns`: each visible column with its `count`, `wip_limit` and `wip_scope`.
  - `blocked` and `stale`: counts of open items.
  - `done_this_week` and `rework_this_week`: since `since`, seven days back. Items brought in by the backlog import never count. Off the board's home, where the history isn't, both are null and `home` names the computer holding the board.
- **`releases`:** the current package and the last two, newest first.
- **`team`:** each bot of the project (`bot`, as in `list_bots`), its items in Doing, and its open task count.
- **`meetings` and `action_items`:** empty until meetings land.
