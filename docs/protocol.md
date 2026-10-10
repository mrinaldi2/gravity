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

A request whose handler fails unexpectedly (a panic in the daemon) is answered
`internal` under its own `req_id`, JSON or binary, whether its handler answers
at once or later; the connection goes on serving. A connection the daemon can
no longer serve is closed, never left open and silent, so a client reconnects
(H-170). Requests from a linked computer get the same `internal` answer.

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
| `item_comment {id, body, reply_to?}` | control | `edited`: `done` (the item) and `told`, the bots told. The owner's comment is delivered, once each, to every bot whose card question it answers (see owner threads), the card's assignee and the project's lead, as a **note** from the owner: `[card <id>] Owner replied to your question on the card: <body>` (it replies to the bot's question), `… Owner commented on the card you asked about: …`, or `… Owner commented on the card: …`, each followed by how to answer on the card (`item_comment` with `reply_to` the comment's id). A note, not a chat, so the bot answers on the card and its turn isn't posted to the owner's thread (H-201 S2, H-211). One recipient failing doesn't stop the others. A bot's comment (MCP) tells no one. |

Cards carry `owner_commented_at`, the owner's latest comment on them (H-211):
a computer mirroring the board closes its card questions asked before it.

Bots read a card's comments in MCP `item_get`: `comments` lists the latest 50,
oldest first, each `{id, author, author_name, body, reply_to, at}`. `author` is
stored form (`user`, `device:<id>`, `bot:<id>`); `author_name` is `owner` for
the owner on any client, else the bot's name (H-201).

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

A paired device's `hello` may carry `"app_version"` and `"app_build"` (the iOS app
sends both from H-230). The daemon keeps the latest per device with when it saw them,
each trimmed and kept only at 1–32 printable ASCII characters, and shows them as
`InstallDevice.app_version` (H-241). They are display only: a reported version never
creates or confirms a deploy record. Other connections' fields are ignored.

A client may add `"features": ["permission_cards"]` to `hello`: it shows bots'
permission prompts and can answer them (`answer_permission`). The daemon only holds
a Claude Code prompt for the app while at least one such client with the `control`
grant is connected; otherwise the prompt stays in the bot's terminal.

Terminal cards (an owner command waiting on the owner, bot id `terminal`) need a second
feature, `"terminal_card"`, which the desktop app sends from 0.16. A client without it
never sees one: not in `permission_request` / `permission_resolved` pushes, not in
`list_permissions`, and `answer_permission` on one is refused with `forbidden`. With no
connected client that sends both features, `owner_request` is refused as if no app were
open.

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
| `input` | `bot_id, data` (utf8 string, may contain control chars) | none (fire-and-forget; requires `control` grant). For a linked bot, an `error` `forbidden` with "Do it on <computer> or your phone…", and nothing is sent (H-303) |
| `resize` | `bot_id, cols, rows, force?` | none (requires `control` grant) |
| `send_user_message` | `to_bot_id, body, item_id?` | `message` (with `item_id`, `commented`) |
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
| `browser_input` | `bot_id, tab_id, event` | Fire-and-forget, as `input` is: no reply, and an `error` (null `req_id`) only when the event cannot be delivered. The owner's mouse or keyboard on a tab this daemon is streaming (`watch_browser`), to sign a bot in or get it past a page. `event` is one of `{ kind: "mouse", action: down\|up\|move, x, y, button: left\|middle\|right\|none, clicks, modifiers }`, `{ kind: "wheel", x, y, dx, dy, modifiers }`, `{ kind: "key", action: down\|up, key, code, key_code, text?, modifiers }` (`text` is what a key going down types) or `{ kind: "text", text }` (a paste). Coordinates are the page's CSS pixels, the `width` and `height` of `browser_frame`; `modifiers` is Alt 1, Ctrl 2, Meta 4, Shift 8. For a linked bot it is refused with an `error` `forbidden` (the request's `req_id`) saying "Do it on <computer> or your phone…", and nothing is sent (H-303). Requires `control` |
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
           "options": [{"key","label","description?","grants?": [{"bot","extra"}]}], "recommendation?",
           "raised_by": {"bot_id","name","avatar"}, "on_behalf_of_bot_id?",
           "origin_chain", "source_message_id?", "source_task_id?",
           "priority": "normal"|"urgent", "deadline_at?",
           "state": "open"|"answered"|"held"|"settled"|"withdrawn", "held_until?",
           "ruling": {"option?","text","reason?","answered_at","answered_by"}?,
           "published_at?", "supersedes_id?", "superseded_by_id?", "withdrawn_reason?",
           "tags": ["..."], "comment_count", "last_comment_at?",
           "comments": [...], "notifications": [...], "edited_at?", "created_at" }
  An option's `grants` (H-117) are permission extras the owner grants by picking it.
  - **At raise:** each one is checked. `bot` may be given as a name and is stored as the bot's id; `extra` must be a known extra.
  - **When they apply:** when the owner's own ruling is published, with no second step. A ruling a bot relayed applies on the owner's `confirm_decision`.
  - **Who may rule on one:** only a client whose hello lists `decision_grants`, which shows the grants. Views carry each such option's `grants_sha` (the sha256 of its `grants`, computed, never stored).
    - `answer_decision`, `update_decision` with `ruling_option`, `publish_decisions` items and `confirm_decision` on a granting option must send back that `grants_sha`. A mismatch gets `conflict` (the grants changed since they were shown).
    - Other clients get `forbidden`: "Answer this on the desktop — this choice changes bot permissions".
    - `confirm_relayed` skips granting ones (listed as `failed`); confirm those one at a time.
  - **Not over a peer link for now (H-301, owner ruling 6df14f7a):** a ruling's grants for a bot on a linked computer aren't sent. Each is said on the decision with "set it on <computer> or from your phone, in the app there under <bot>'s permissions". A `grant_extras` that arrives anyway is refused with "Approve on <computer> or your phone", and nothing is granted. What follows describes that path, kept for signed approvals, and it is off.
  - **What they do:** they only add extras. A linked bot gets them on its own computer, through peer request `grant_extras {bot_id, extras, decision}`. It is accepted only for bots exposed to that peer and only for `install`, `daemon_restart` and `quiesce` (ARCH-R51 S1c), and it is recorded in that computer's `owner_action_audit` (as `grant:<bot id>`, event `granted`).
  - **Per grant, not per frame (H-163):** the ruling's computer sends only those three. Each other extra is refused there, on the decision: "Not granted on <computer>: <bot>'s <extra> can't be granted from another computer; set it on <computer>, in the app there under <bot>'s permissions." The receiving computer checks the same way: it applies what it may and answers `{extras: <now held>, refused: [{extra, why}]}`. When nothing is grantable, it refuses the request (`forbidden`).
  - **Until it lands (H-163):** the ruling's computer keeps each linked bot's grant in `peer_grant` until that computer applies or refuses it.
    - It is sent at once, again on link-up, and on a 60 s sweep.
    - Only a transient failure is retried: the peer offline, its link closing mid-request, or no answer. Any answer is final.
    - **Bound to its ruling (CE-020 M1):** each row keeps the picked option's `grants_sha` and `decided_at`. Before every send, the decision must still be settled, not superseded (reopened or replaced; a withdrawn one isn't settled), and on the same option sha. Otherwise the row ends `cancelled`.
    - **The bot's computer has the last word (M2):**
      - the frame carries `decided_at`;
      - `set_bot_permission_extras` stamps the bot (`bot_extras_changed`);
      - a grant decided before the owner's last change there is refused (`conflict`, "the owner changed its extras here after the ruling; set it here").
      - A row still waiting 24 h after it was filed ends `expired`.
    - The decision gets a comment for each outcome:
      - "Granted on <computer>: <bot> now has …";
      - "Not granted on <computer>: <bot>'s <extra> (<why>); set it on <computer>, …", one per refused extra;
      - "No longer granted on <computer>: the ruling changed, …";
      - "Not sent: <computer> was unreachable for a day, …; set it on <computer>, …";
      - "Waiting for <computer> …", said once while it is unreachable.
  - **Record:** a comment on the decision lists what was applied.
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

- **The app's identity (H-110)** is compiled into hermesd; nothing is pinned in a file a bot could rewrite. A T4 `secrets/owner-app.json` is deleted at start.
  - macOS: the code requirement `anchor apple generic and identifier "com.manuelrinaldi.thehermes" and certificate leaf[subject.OU] = "<Team ID>"`. The Team ID comes from `HERMES_TEAM_ID` at build time; without it a placeholder no app matches, so the app falls back to `client.token` in phase 1. A macOS hermesd with the placeholder ignores `bot_bearer = "refuse"`, logs an error and stays in phase 1, and the release build (`prepare-sidecar.sh`, `release.yml`) refuses the placeholder unless `HERMES_DEV_BUILD` is set (ARCH-R38).
  - Windows: an executable directly in `<Program Files>\The Hermes` (from `SHGetKnownFolderPath`, not the environment), other than `hermesd`. The setup installs per machine, so only an administrator can change that folder, and asks before installing anywhere else. The logon task runs `hermesd.exe` from that folder too, not the home's copy; the app clears every `WEBVIEW2_*` variable, limits DLL loading to System32 and its own folder, and has no devtools in a release build, all before its first webview (ARCH-R38).
  - A build with `HERMES_DEV_BUILD=1` (`scripts/dev.sh`) never enforces phase 2 storage, so an unsigned dev app keeps its `client.token`.
- **Where to connect:** the daemon writes its endpoint address to `<home>/run/endpoint`.
- **Methods.** A client sends one JSON-RPC request on the endpoint, before any bot traffic, and the connection closes after the answer.
  - `hermes/owner_ticket`: the daemon checks the caller against the app's identity: on macOS the peer's audit token against the requirement, on Windows the peer's executable path. A caller that passes gets `{"ticket": "…"}`.
  - `hermes/owner_request {"command": "…", "cwd": "…"}`: the daemon raises a permission card filed under bot id `terminal` (tool `Terminal command`) and waits for the owner's answer. If the owner allows it, the caller gets a ticket.
  - The card's `origin` (also its `input`) carries separate fields (UX-014): `command` (no pid), `pid`, `process` (the pid's executable name), `launched_from` (the nearest ancestor that is an app or terminal: Terminal, iTerm2, Code, claude…), `cwd` (read from the OS; the client's `cwd` only where the OS can't tell, i.e. Windows) and `bot` (the name of the bot whose workspace holds `cwd`). A field that can't be read is left out. The client composes every line and never shows `summary`.
  - The app answers these cards with `allow_once` ("Allow this command") or `deny` (no reason); it offers no "Allow for session" and no single-key shortcuts for a terminal command.
- **Tickets:** single use, valid for 60 s. A ticket is passed as the `token` in WS `hello` and grants the same capabilities as the client token.
- **Observability (H-165):** the daemon logs `owner ticket granted` and `owner ticket redeemed` at info, with `via` (`app`, `cli`), the grantee's `pid` and `exe` path, and the compiled `team` on a grant; a hello on `client.token` logs `owner connected with client.token, not a ticket`. No line carries a ticket or a token. `/health` counts, since the daemon started, `owner_tickets_granted`, `owner_tickets_redeemed` and `client_token_fallbacks` (owner hellos on `client.token`).
- **Refusals:** error `-32002`.
  - A caller inside a bot session is always refused: "a bot can't act as the owner" for `owner_ticket`, "Commands that act as the owner can't run from a bot's session." for `owner_request`.
  - Other refusals: "not the owner's app"; for `owner_request`, which the CLI prints as is, "You denied this command in The Hermes. It did not run.", "No answer in The Hermes, so the command did not run." and "Open The Hermes on this computer to allow this command, then run it again." (no app is connected to show the card).
- **Clients:** the desktop app asks for a ticket on every connect. `hermesd` CLI owner commands ask the owner to allow them, printing "Asking for your OK in The Hermes app…". Both fall back to `client.token` only when the daemon has no endpoint (a daemon older than T4). The daemon accepts `client.token` until T6.
- **Approve:** only a connection with the `approve` grant can answer a `terminal` card. Other connections get `forbidden`. Bot cards still need only `control`.

### Secrets at rest (T5)

- **Device tokens:** a newly paired device is stored as `secrets/device-<id>.sha256`, the sha256 of its token. The phone holds the only plaintext. Older `device-<id>.token` files still work.
- **Phase 2:** this is `[auth] bot_bearer = "refuse"`, set per machine. At start, the daemon:
  - hashes each old device file and deletes the plaintext;
  - deletes `client.token` and every `bot-*.token`;
  - never writes those files again.

  The owner then connects only with a ticket (T4).
- **Rolling back from phase 2:** the owner has to re-pair phones and run `hermesd service install` again, which writes a new `client.token`.
- **Peer tokens:** `[auth] peer_tokens = "file"` (the default) or `"keychain"`.
  - With `keychain` on macOS, `peer-*.token` files move into the login Keychain at start. The Keychain item's ACL trusts only the hermesd binary that created it.
  - hermesd is ad-hoc signed, so after each update macOS asks once per link before the daemon may read it. Turn `keychain` on once hermesd is code-signed.
  - On Windows and Linux the setting falls back to files. This is a known residual.

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

- **Planned (H-137):** the lead or DevOps calls `release_plan {name, display_version?, items, changelog?}` as soon as a release's contents are decided. Its items may be in any open column, and each item is in at most one open package (planned included).
  - `release_items {release_id, add?, remove?, reason?}` changes its scope until it is submitted. A planned package takes any open item; one being assembled takes items in Verify only. Once it has a build or a test result its items are fixed (cancel it and package a new one). It can't be left empty.
  - `release_assemble {release_id}` moves `planned` → `assembling` once every item is in Verify. Builds, `release_test`, submit and the frozen hash then work as above, unchanged. Builds and installer builds are refused while it is planned.
  - Each step is a release event: `planned {items}`, `items_changed {added, removed}` with the reason as its note, and `assembled`.
  - The lead may cancel a planned package; anything later is DevOps's.
- **Progress:** every release carries `plan` and `readiness`. They are live, and never part of the frozen hash.
  - `plan` is `[{item_id, title, column_key, category, assignee, blocked, ac_checked, ac_total, ready}]`, where `ready` means Verify or later.
  - `readiness` is `{items_total, items_ready, builds: [platform], tests_required, tests_passed}`.
- **Successor:** after a mixed ruling (`repackaging`) or a failed deploy (`partially_deployed`), DevOps calls `release_create` with `from`. The predecessor's shipped items join straight from Owner testing, and the predecessor becomes `superseded` when the successor is **submitted**. The successor is a new build, so it gets a new ruling. A failed deploy clears its items' `release_id`.
- **Cancel:** `release_cancel {release_id, reason?}` (DevOps; the lead for a planned one) removes a package that is still `planned`, `assembling` or `built`; anything submitted or later is refused. Its items were never moved: a predecessor's shipped items stay in Owner testing for the predecessor, which can take a new successor, and items from Verify are free again. A `cancelled` event keeps who, why and what it held, and shows in the predecessor's `events`.
- **Deployed via (H-121, H-191):** `release_deployed_via {release_id, via_release_id}` (DevOps or the lead) closes a package the owner ruled to ship that a later, deployed release contains, for when the owner skips installing it or a later release replaced it part way through its install.
  - The old package must be `approved`, `deploying`, `paused` or `partially_deployed`. A deployment still open on it doesn't stop it.
  - Every computer it was to reach (`deploys_to`) and wasn't confirmed on must be one the via package reaches (its own `deploys_to`, or a computer it was confirmed on). A confirm later undone by a rollback on that computer doesn't count as confirmed.
  - The via package must be `deployed`; it may itself have been closed this way, so a chain works.
  - The via package must contain the old one:
    - the old package's recorded source commit is the via's, or in its history. The daemon checks this with `git merge-base --is-ancestor` in its own copy of the project's repository: a bare, blob-less clone under `<home>/cache/repos/<project>.git`, cloned and fetched with no hooks and no user git config.
    - A package from before commits were recorded is contained when the via package is newer and has a build for every platform it built.
  - Its post-install acceptance criteria must be ticked (H-116).
  - It records a `deployed_via` event (note `deployed via <name>`, or `deployed via <name> on <computers>` when it was installed on some itself; detail `{via_release_id, basis: "ancestry" | "release_branch", commit?, via_commit?, installed, covered, called_off}`), creates no deployment rows, sets the status to `deployed` and moves its items to Done. Each deploy still open on it is called off: its row gets result `superseded`, its deploy task is cancelled with a note to the tester, and a `deploy_confirm` for it is refused. `superseded` is never a result a tester reports.
- **Replaced by a deploy (H-191):** when a deploy confirm completes a package (`deployed`), every older open package of the same platform family (iOS packages with iOS, the rest together) is closed:
  - one nobody ruled on (`awaiting_owner`, `held`, `repackaging`) becomes `superseded`, with a `superseded` event, only when the deployed package contains it and reaches every computer it was for: the same containment and coverage checks `release_deployed_via` makes (a later hotfix cut from an older branch, or a Mac-only package, doesn't answer its ruling). Its decision is withdrawn, so Needs you drops it; its items the deployed package doesn't hold go back to Verify with no `release_id`. If a check fails it keeps its ruling, with a `not_closed` event saying why.
  - one ruled to ship whose install got under way (`deploying`, `paused`, `partially_deployed`) is closed through the deployed one as `release_deployed_via` would. If any check fails it stays open, with a `not_closed` event saying why.
  - An `approved` package never installed is left for DevOps or the lead to close with the tool.
- **Testing:**
  - DevOps fills `changelog` and `how_to_test` (`[{item_id?, platform, steps}]`) with `release_update` while the package is assembling.
  - Required machines are per computer, not per platform (H-115). Where a tester tests:
    - A linked tester tests on its peer's computer, whatever its role says (ARCH-R55 S2).
    - A tester here tests on the machine its role names, unless that is a linked computer's name.
    - Otherwise it tests on this computer, which is called by its `machine_name`. That defaults to the host's name (the Mac's LocalHostName); the lead sets it with `machine_name_set {name}`, the owner with `release_machines_set {machine_name}`. It can't be a linked computer's name.
  - Each computer's own tester records a result against one of the builds' sha256 with `release_test`. `machine` may be left out (or "") by a tester on one computer; naming another computer is refused. The build must be for that machine's platform: one whose board `required_machines` lists it, or with none configured for it, the items' platforms (a build platform `desktop-mac` is a `desktop` build).
  - `release_submit` is refused until every required computer has passed the current builds. Required computers are the list the owner or lead set, or with none set, every tester's computer. With neither, submit is refused: an empty set is never a pass.
  - A package counts as deployed only once every tester's computer reports a good deploy, unless the **owner** narrowed the list. A lead's list narrows testing only, so no release is "deployed" while a computer runs the old version (ARCH-R55 M1).
  - Both sets are frozen into the package at submit and covered by its frozen hash. The release JSON shows them as `tested_on`, `tested_set_by`, `deploys_to` and `deploys_set_by`. Later edits apply to the next package.
  - `release_machines` (every bot; WS read) returns `{project_id, machine_name, required, deploys_to, set, set_by, testers: [{bot_id, machine}]}`.
  - `release_machines_set {machines?, machine_name?}` sets the list: MCP for the lead (narrowing testing only), WS for the owner (approve, on the board's home; narrows deploys too). Each entry must be a computer some tester tests on, and an empty list goes back to every tester's computer.
  - Migration 033: `release_machine`, `release_machine_setter`, `release_target`, `daemon_name`.
  - **iOS packages (H-176).** A package whose every build is `ios` freezes its own targets: it is tested on `ios` and deployed to `iphone`, the owner's phone.
    - The tester whose role names `ios` (iOS QA) reports its `release_test`, and gets the `iphone` deploy task and confirms it.
    - `ios` and `iphone` never count as desktop computers. The project's list, and `required` and `deploys_to` in `release_machines`, stay desktop-only, so desktop packages keep needing every desktop tester's computer.
    - Besides the roles it already assigns, the lead may give, or take away, a `tester` role on `ios` with `role_set {bot, role: "tester", machine: "ios"}`. That's only on `ios` (not `iphone`), and only for a bot that isn't the lead, DevOps or a dev (separation of duties) and isn't already testing a desktop computer. Nor may the lead later make that iOS tester a dev (or DevOps or lead): taking the tester role back comes first. Every other tester, DevOps or lead role stays the owner's (ARCH-R30).
    - A boot repair (`release::ios_repair`) re-freezes, to `iphone`, the deploy targets of iOS packages submitted before this change, and rehashes them so their deploy passes the frozen-hash check. That applies only to packages still as frozen, not over, whose deploy targets nobody set and that haven't started a deploy. Each repair is a `targets_repaired` release event on the package (actor `daemon`, `detail {from, to}`); a later boot finds none.
    - **One iOS target (H-231).** `ios` and `iphone` name the same target: a deploy sent or confirmed under either reaches a package frozen with the other, and a role naming either tests iOS as `ios`.
      - A tester role on the iOS target counts as `ios` wherever its bot runs, even a bot on a linked computer, whose other roles count as that computer. So the board's home finds a linked iOS tester for an `ios`/`iphone` deploy, and an iOS tester never gets the desktop deploy of the computer it runs on.
      - A boot settle marks deployed each iOS package still deploying whose targets are all reached under the one name (one confirmed on `ios` while it waited for `iphone`), its items to Done, with a `targets_settled` event (actor `daemon`). A package with post-install criteria unticked is left.
- **Hold:** `release_hold` keeps the decision open and holds it (`remind_at` becomes its `held_until`). When the reminder comes due, the decision sweep resumes it and the package goes back to `awaiting_owner`. `release_unhold` does the same on request.
- **Pause:** `release_pause` (owner, or DevOps over MCP) pauses a rollout in progress. Deploys and installs are refused with the reason, and every tester holding an open deploy task gets a note. `release_resume` returns it to `deploying`.
- **Who may rule:** `can_rule` says whether this connection may rule (the approve grant, on the board's home). When it can't, `rule_on` names the home computer. `BoardSnapshot.can_rule` carries the same flag.
- **Installing a release (B8, H-020 §2.6, ARCH-R43):** a tester runs `hermesd release install <release> [--dry-run | --status]` from their own session.
  - **Gate.** It calls `install_release` over the local endpoint, so the daemon knows the bot by its process and checks the gate: the owner's settled approval, an open deploy task for this tester, and the frozen hash. Off the board's home, the call is forwarded there.
  - **Builds.** It takes this computer's builds: `desktop` on macOS and Windows, else `daemon`. For iOS it only prints the `install_url`.
  - **Stage.** Each build is copied from the home's own file, or downloaded from its HTTPS `url` with `curl`, into a fresh private folder (`hermes-install-<release>-<random>` under the system temp dir, mode 0700, never reused). Its sha256 is checked there; a mismatch stops the install.
  - **Signature.** Before anything is swapped in, the unpacked build is checked against the identity compiled into hermesd (H-110):
    - **macOS:** `codesign --verify --strict --deep` against the app's code requirement, or against the owner's team for a bare hermesd. The copy staged in `/Applications` is checked again before the rename.
    - **Windows:** Authenticode (`Get-AuthenticodeSignature`, which is WinVerifyTrust) must be valid and signed by the `HERMES_WINDOWS_SIGNER` compiled in.
    - **When it can't check:** a dev build, a build without a Team ID, or Windows releases while they're unsigned. It prints `signature check skipped: <why>` and never skips silently. A failed check stops the install.
  - **Install (H-117 X1).** For an app, the swap is the system job's, never the bot's session. The job runs a copy of this hermesd as `hermesd release apply-app <release> --app <staged bundle> --version <v>`, which:
    1. copies the bundle beside `/Applications/<app>` and checks its signature there again;
    2. keeps the app it replaces as `<home>/backups/app/<app>`, the one previous app, and remembers that app's bundle hash in its own memory (ARCH-R52 M2);
    3. swaps the new one in and runs its `service install`;
    4. applies the boot gate: it waits up to 120 s for `/health` to answer with the new binary's `binary_sha256`. The version isn't enough, because an old daemon at the same version would pass (ARCH-R52 M3). If the gate fails, the backup goes back, but only if its bundle hash matches what was kept and its signature checks out. Then its `service install` runs and the job exits non-zero. The daemon's boot check records the install as rolled back.
    5. If putting the backup back fails too, `service install` runs from `<home>/bin/hermesd` (else from the app in place), and `<home>/run/rollback-failed.json` is left. At boot, the daemon pushes an error notice to the owner and files a Run card that runs `service install` from `/Applications/<app>` (ARCH-R52 S3).
  - **`/health`** reports `binary_sha256`, the sha256 of the daemon's own binary.
  - **Allowed.** The `install` extra allows `hermesd release install` with no owner prompt (H-166).
    - The rule names this daemon's own binary by its exact path, in each way a bot quotes it, for example `Bash("/Applications/The Hermes.app/Contents/MacOS/hermesd" release install *)`. Bots have no `hermesd` on their PATH, so a rule for a bare `hermesd` never matched, and the classifier refused the install. A bare name would also allow whatever `hermesd` comes first on the bot's PATH.
    - Only the arguments after the subcommand are a wildcard. A path holding `*`, a parenthesis or a quote gets no rule.
    - `install_release` answers with `command`: the exact line to run on the caller's own computer. Off-home, the command uses that computer's binary, not the home's.
  - **Hand-off.** `service install` restarts the daemon and every bot, this session included, so it's handed to the system:
    - a one-shot launchd job on macOS (`com.thehermes.release-install.<release>`, `RunAtLoad`, not kept alive);
    - a one-time scheduled task on Windows, which runs the setup with `/S`; the setup's own hook installs the service;
    - a detached process group elsewhere.

    The job writes `<home>/logs/release-install-<release>.log` and its exit code to `<home>/run/release-install-<release>.status`, then removes the stage and itself. The command exits at once.
  - **After the restart.** From the next session, `--status` prints how it ended and the end of the log. The open deploy task in the session's resume note is the reminder. The tester smoke-tests and reports with `deploy_confirm`. `--dry-run` stops after the checksum and signature checks.
  - **Who may replace a running service (H-193).** Opening the app never replaces a daemon that answers on its port:
    - Its launch repair (`service install --no-migrate`) runs only when the files say the service is broken (binary missing, or an interrupted switch-over) and nothing answers `/health`.
    - A newer bundled daemon is offered as the "Update Hermes service" toast, a click.
    - Otherwise the service is replaced only by a release install, with its quiesce.

    Runbook: to find out what restarted the service, read `<home>/logs/service-install.log`. Each `service install` appends a line with the time, version, arguments, the executable that ran it (with its pid), and what launched that executable.
  - **Deny rules, advisory only.** Every bot but the project's DevOps gets deny rules in its generated Claude Code settings for direct installer and service commands: `hermesd service install/uninstall`, `installer -pkg`, `msiexec`, `xcrun devicectl device install`, `ios-deploy`, and a silent `*-setup.exe /S`.
    - They are **advisory, not a boundary**. They match command text, so a script, an alias, a renamed binary or a Codex-runtime bot gets past them.
    - What enforces the gate is the daemon: `install_release`, the approval, the frozen hash and the signature check. Every deploy carries its task and decision ids for the audit trail.
- **Landing on main (H-117 X2):** DevOps runs `hermesd release land <release> [--commit <sha>] [--branch <b>] [--tag <t>] [--dry-run]` from its checkout. The `release_main` extra allows exactly this command.
  - **Daemon gate.** The command asks the daemon over the local endpoint (`hermes/release_land`). It needs the `release_main` extra, the project's DevOps role, and a release the owner approved (`approved`, `deploying`, `partially_deployed` or `deployed`).
  - **Commit (ARCH-R52 M1).** Only the commit the release's builds record (`source_commit`) lands, and the gate returns it. A release whose builds don't all name it, or that come from different commits, is refused. `--commit` may only restate it. The release branch (`release/desktop-<version>` unless named) must still be at that commit, so code pushed after the builds never lands.
  - **Push.** `main` must fast-forward to it; nothing is ever forced, and a diverged main is refused before anything is pushed. Then `main` and the annotated tag (`desktop-v<version>` unless named) are pushed. Git runs with hooks off (`core.hooksPath=/dev/null`).
  - **Transport.** Over the remote as configured. When SSH has no key, it goes to GitHub over HTTPS with gh's credential, for that one command; no git config changes.
  - **Check.** After the push, `git ls-remote` of the project's configured repo (the checkout's origin if none is configured), run outside the checkout without its git config or the user's, must show main and the peeled tag at the commit (ARCH-R52 S1).
  - **Record.** `hermes/release_landed` records a `landed` event, with the commit and tag, on the release. Any other commit is refused.
  - A raw `git push … main` still goes through the guard, unchanged.
- **Building the Windows installer (H-117 X3, H-104):** Tester Win runs `hermesd release build-installer <release> [--commit <sha>] [--script <path>] [--output <file>] [--timeout <minutes>]` in its release worktree. The `build_installers` extra allows exactly this command, never the script, which the bot could edit.
  - **Daemon gate.** `hermes/release_build_installer` needs the extra and a package still open for builds (assembling or built), and returns the commit its builds record, if any yet.
  - **Commit.** It builds that commit (`--commit` may only restate it), else the release branch's tip.
  - **Fresh worktree (ARCH-R52 S2).** It builds in a fresh `git worktree add --detach` of that commit in a private temp folder, with hooks off. There the tree must be clean, with untracked files and any file marked assume-unchanged or skip-worktree refused, and the script (`scripts/build-nsis.ps1` unless named) must match its blob.
  - **Run.** It runs `powershell -NoProfile -ExecutionPolicy Bypass -File <script>` with a time limit (45 minutes by default). The newest `*-setup.exe` it makes (or `--output`) is copied to `<checkout>/target/release-installers/`, and the temp worktree is removed.
  - **Record.** The installer is hashed, and `hermes/release_installer_built` records an `installer_built` event `{commit, file, sha256}` for `release publish`. Any commit but the recorded one is refused.
- **Source commits (ARCH-R52 M1):** `release_attach_build` and `release_publish` take `source_commit` (40 lowercase hex). `hermesd release publish` sends `--source-commit`, or the checkout's HEAD when the tree is clean.
  - The commit is stored with the build (migration 031), shown in the release JSON, and folded into the frozen hash when present.
  - Every build of a release must come from one commit; a build from another is refused.
- **Grants from linked computers (ARCH-R51):** `bot_grants {bot_id}` (read) answers `{type: "bot_grants", grants: [{at, from, extras, decision}]}`. These are the extras rulings on a linked computer granted the bot here, newest first, and the app lists them under the bot's extras.
- **Serving builds (H-020 §6.6):** `release_publish {release_id, file, platform?, version?, bundle_id?}` (DevOps with the Publish extra; `hermesd release publish <release> <file>` calls it with the bot's token) copies a build into the served directory, `[releases] dir` (default `<home>/releases`), at `<release_id>/<platform>/<file>`.
  - The file must be an `.ipa`, `.zip`, `.dmg`, `.exe` or `.msi`, and must resolve, symlinks and all, inside `[releases] source_roots` (default: the `<repo>-rel-*` release worktrees in the trusted paths) or the project's artifacts. It is opened once, without following a link at the end of its path (`O_NOFOLLOW`; on Windows the reparse point itself), and the handle must be a regular file with one name (no hard link). The handle's own real path (`F_GETPATH`, `/proc/self/fd`, `GetFinalPathNameByHandleW`) must still be the checked path, inside a source root, so a directory swapped for a link after the checks is refused (H-100). It is hashed and copied from that handle. Inside the served directory, symlinks are refused; copies are staged beside it, never in it.
  - The served directory must be a real folder of its own: a symlink, a folder that is or contains the daemon home, its secrets or `bus.sqlite`, or any folder inside a home other than the dedicated `<home>/releases` (e.g. `<home>/projects`), is refused at config load and at every publish. The owner sees it in Needs you (`serving_off`), and a refused publish also pushes them a notice (H-100). Only the daemon writes there: every bot's guard and settings deny writes to it (CE-010).
  - A published file is never replaced by a different one.
  - It attaches the build with its sha256, its `url` under `[releases] base_url`, and its `install_url`. For an `.ipa`, that is the `itms-services://` link to a generated `manifest.plist`, and an `index.html` install page with one Install button is written beside it (H-229); for other builds, it is the `url`.
  - `install_release` re-hashes every served build against the frozen sha256 and returns `verified` per build. On a mismatch, it refuses the install, pauses a rollout in progress and tells DevOps.
- **Installing on an iPhone or iPad (H-229, UX-043):** the messages are `hermes.home.v1` `ReleaseInstall*` and `InstallOffer*`, as proto3 JSON with the proto field names; a field at its default is left out. All four need the board's home.
  - `release_install {release_id, check_site?}` (read) answers `{type: "release_install", install}`: the package's version, its iOS `build`, `state`, `installable` (awaiting_owner, approved, deploying or deployed), `for_testing`, the HTTPS `page_url` (the build's `index.html`, written on demand for builds published before it existed), the `install_url`. Both are derived by the daemon from `[releases] base_url` and the build's served folder, never taken from the build's `url` or `install_url`: the build's file must lie at `<served dir>/<release>/ios/<file>` beside its `manifest.plist` and still match its sha256, else both are empty and nothing is written, probed or sent (CE review of H-229, M1). It also answers `computer`, `approved_at` and the paired `devices` (not revoked, last seen first, `connected` when one holds a live connection, and `app_version` "0.6.1 (12)" with `app_version_seen_at` as the device last reported it). With `check_site`, the daemon first asks for the page over HTTPS (curl, 4 s) and sets `site {serving, problem, checked_at}`.
  - `release_send_to_device {release_id, device_id}` (control, from the app's ticket or a paired device only, never the owner token; M2) answers `{type: "install_offer", offer}`: `title` ("The Hermes 0.6.1 (12) is ready to install"), `body` ("Approved on 8 Oct 2026. Tap to install." or "Ready to test. Tap to install."), `install_url`, `page_url` and `delivered` (the device was connected). It is kept as that device's one waiting offer and pushed as `{type: "install_offer", device_id, offer}` to that device's connections only. Refused (`conflict`) when the package isn't installable or has no published iOS build, and `not_found` for an unknown or revoked device.
  - `install_offers {}` (read) answers `{type: "install_offers", offers}`: the asking device's waiting offer, at most one; none on a connection that isn't a device. `install_offer_dismiss {release_id}` (control, a device only) drops it (Not now) and answers the same.

## Pausing every project for an install (H-117)

An install replaces the daemon. While it runs, **quiesce** holds every project on that computer still, and resumes them after.

**The pause.** A pause is one open row in `quiesce`. It is in the database, so it outlasts the install's daemon restart. While it is open:
- **Bots:** the supervision tick stops every session and starts none. A stopped bot shows `Stopped` with the reason `Paused (install of <release>)`.
- **The installing bot** (`exempt_bot`) is the one exception: it keeps running until its install is handed to the system, which then stops everything with the daemon (ARCH-R49 M1).
- **Routines:** the scheduler records due routine slots but runs nothing, and expires nothing.
- **Messages:** deliveries wait in the queue, local and peer alike.
- **Workers:** the worker queue places nothing. A running worker is a bot, so it is held and comes back with its task still open.

**Resume.** Resuming:
- skips all but each routine's latest recorded slot, so a routine fires at most once for everything it missed;
- extends open tasks' deadlines by the time paused;
- restarts the services the pause stopped;
- lets everything run again.

If the pause stays open past `deadline_at` (30 minutes by default), the daemon resumes by itself, tells the owner, and notes each project's DevOps and lead.

**What a pause records.** The `quiesce` object: `{id, reason, release_id, started_by, started_at, deadline_at, phase, report, services_stopped, resumed_at, outcome}`.
- `report.paused` counts what was held: `{bots, routines, workers, queued}`.
- `report.resumed` records how it ended: `{at, outcome, paused_seconds, coalesced_runs, extended_tasks, services_started}`.
- `outcome` is one of `resumed`, `install_ok`, `rolled_back` or `deadline`.

**Reaping (Q3).** `quiesce start` pauses, then reaps what bot sessions left running.
- **What is signalled:** only processes the lineage ledger, a `THEHERMES_SESSION` environment tag or a session's Job Object ties to a session. The installing bot's processes are spared.
- **How:**
  - each process is re-checked by `(pid, start)` just before every signal;
  - a group is signalled whole only when every member is the sessions' own;
  - `TERM`, then `KILL` after 5 s;
  - on Windows, the session's job is terminated.
- **What is never done:** nothing is signalled by name or pattern.
- **Services.** It then stops the services from `[[quiesce.service]]` that are running **and** hold the home (or are marked `always`), using their own stop command. The command's program is looked up in `[quiesce] search_path`, never `PATH`, and runs with a 120 s timeout.
  - The defaults are colima (`brew services stop colima`) and lima (`limactl stop default`).
  - VM processes are never signalled.
  - Resume starts again only what the pause stopped.
  - The list is read once at daemon start. If `hermesd.toml`'s list changes after that, the change isn't used, and the report and the app's banner say so (`services_changed`).
- **What still holds the home** goes into `report.unresolved` as `{pid, command, cwd, path, project_id?, bot_id?}` and is never signalled. The install goes ahead only when the list is empty (phase `ready`, else `blocked`). A later `start` for the same release takes up the open pause again.

```toml
[quiesce]
deadline_minutes = 30
search_path = ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/bin"]

[[quiesce.service]]
name = "colima"
detect = "brew services info colima --json"
running_match = "\"running\": true"
holder_match = ["limactl", "colima", "com.apple.Virtualization.VirtualMachine", "qemu-system"]
stop = "brew services stop colima"
start = "brew services start colima"
```

**Installing (Q4).** The pause is the installing bot's to ask for, on its own computer only. It is the MCP tool `install_quiesce {release_id, action: start|status|resume, version?}`, also run as `hermesd quiesce start|status|resume <release>` over the local endpoint, which knows the bot by its process. The daemon allows it only when all of these hold:
- the bot holds the `quiesce` extra, granted once by the owner ("Pause all projects for an install"; it allows this binary's `quiesce` by its exact path, as for `release install`);
- it is the project's DevOps, or holds the `install` extra;
- it holds an open deploy task for an approved release, the same gate as `install_release`, or (H-166) it is the project's DevOps and the release has a deployment or rollback under way on this computer (by its machine name).

A refusal says which of these is missing.

**Where the board lives on another computer (H-166).**
- The tester's own daemon handles `install_quiesce`. It checks the bot's extras there, asks the home for the gate (`install_release` as the stand-in), and then pauses its own computer.
- The home refuses a forwarded `install_quiesce` ("doesn't pause itself for another computer's install"). An older peer that still forwards the tool gets that refusal, not a pause of the home.

**Install pending (H-166, H-288).** While a bot on this computer holds an open deploy or rollback task that installs **here**, or while a pause is open, the daemon refreshes `<home>/run/install-pending.json` (`{why, release_id}`) every 15 s. It removes the file when neither holds.
- **What installs here:**
  - **The target** is the task's deployment machine, kept in `task_target` (migration `TASK_TARGET`). It's sent to a linked computer in the task frame as `target`, and the receiver stores its own name, or `ios`/`iphone`.
  - **Not here:** an `ios` or `iphone` target installs on the phone, and a linked computer's name installs there. A task with no target (DevOps asked to roll back, or a peer older than H-288) installs nothing by itself.
  - **A confirmed install** isn't pending, even while its tester still holds the task.
- **When a package becomes `deployed`,** its installs nobody confirmed (for example the `iphone` row of an iOS package confirmed through its `ios` row) are called off as `superseded`. Their testers' tasks are closed with a note, so no stale task keeps the flag.
- The guard refuses `colima start`, `limactl start` and a VR run (`vr-ci.sh`) while the file is fresh, saying why. A VM sharing the home blocks the install.
- A file older than 2 minutes means the daemon stopped refreshing it, and is ignored.
- Bots learn this from the shared-computer section of their prompt. `start` pauses with the caller as `exempt_bot`, reaps and answers `{quiesce, report, proceed}`. Its report is recorded on the release as a `quiesce` event, and each lead gets a note.

`hermesd release install <release>` runs `start` itself before it stages anything:
- **Something still holds the home** (`proceed` false): it prints the holders and stops.
- **Before each build it stages, and again at `install_started`,** it calls `extend`, which moves `deadline_at` a full window on, so the dead-man switch can't resume projects mid-install (ARCH-R50 S2).
- **The install fails before the handoff:** it resumes the pause (`aborted`).
- **Right before the handoff** it calls `install_started` with the sha256 of the daemon binary it installs (none for a Windows setup, which seals its binary). The pause enters the phase `install_started`.
- **The install is handed off:** the system's install job stops the daemon and every session. A daemon that boots with the pause open ends it only in that phase (ARCH-R50 S1):
  - its own binary's sha256 equals the recorded one (or, with no hash, its version equals the one being installed): it resumes as `install_ok`;
  - otherwise the install was rolled back, and it resumes as `rolled_back`.

  A boot before `install_started` (the old daemon restarting before the swap) keeps everything paused. Either way the outcome goes on the release, and leads and DevOps are told. A pause left open by a crash in between resumes at its deadline, and the owner gets a notice.

**WebSocket.**
- `quiesce_status` (read) answers `{type: "quiesce", quiesce: …|null, ended: …|null}`.
  - `ended` is the pause that ended in the past hour. Its `report.resumed` holds `{outcome: install_ok|rolled_back|deadline|aborted|resumed, running_version}`, so the app tells the owner how it went once, even across the install's restart.
  - While blocked, `report.unresolved` lists each holder `{pid, command, project_id?, bot_id?, project_name?, bot_name?}`.
- `quiesce_resume` (approve, the owner's **Resume now**) ends the pause.
- Every change pushes `{type: "quiesce_update", quiesce: …|null}`.

## Owner actions (H-117 R1)

A bot proposes an exact command for the owner to run; only the owner runs it.

**Proposing (MCP).** `propose_owner_action {content, reason, cwd, shell?, target_machine?, item?, decision_id?, pinned_files?, timeout_s?}` answers `{owner_action}`. The bot also has `withdraw_owner_action {id}` and `get_owner_action {id}`. There is no tool that runs one.
- **Defaults and limits:** the shell is zsh on macOS and powershell on Windows; `cmd` takes one line. `timeout_s` defaults to 600 and is at most 3600. `content` is at most 16 KB.
- **Refused:** NUL and control characters (other than newline and tab), C1 controls, bidirectional overrides and zero-width characters, in the content, cwd or reason. A cwd that isn't a folder on this computer is refused too.
- **Pinning:** each pinned file is hashed at proposal. A path the content names that a bot can write (workspaces, trusted folders) and isn't pinned becomes a `flags` warning on the card.

**What the action holds.** It stores `{id, project_id, proposed_by, item_id?, decision_id?, target_machine, shell, cwd, content, pinned_files, reason, timeout_s, sha256, flags, origin, state, created_at, expires_at, run_by?, run_at?, finished_at?, exit_code?, output_tail?, reject_reason?}`.
- `target_machine` is the target daemon's id.
- `sha256` is taken over the canonical JSON of the proposal's fields.
- A trigger refuses any change to what was proposed.
- `state` is one of `proposed`, `running`, `succeeded`, `failed`, `timed_out`, `rejected`, `withdrawn` or `expired` (after 24 h).

**Running it (WebSocket).** The server advertises `owner_actions`, and only a client whose hello lists the feature `owner_actions` gets the cards and pushes.
- `owner_action_list {project_id?}` → `{type: "owner_actions", actions}` (read).
- `owner_action_get {id}` → `{type: "owner_action", action, audit}` (read; audited as viewed).
- `owner_action_run {id, sha256}` (approve) runs it once:
  - The client must render owner actions, and the hash must be the stored one, which is what the client showed.
  - Only the app's one-time ticket or a paired device's credential runs or rejects one, never the owner token, from anywhere (ARCH-R51 M1). On top of that, a loopback connection is traced to its process (lsof), and a process under a bot's session is refused whatever credential it holds.
  - `proposed → running` is a single guarded update, so a second tap gets `conflict`.
- `owner_action_reject {id, reason?}` (approve).
- Pushes: `{type: "owner_action_update", action}` on every change, and `{type: "owner_action_output", id, chunk}` (redacted) while it runs.

**How it runs.** Pinned files are hashed again; a change fails the run before anything runs, listing the old and new hashes. The stored content then runs as one argument, never from a file (ARCH-R49 M2): `zsh -f -c`, `bash --noprofile --norc -c`, `powershell -NoProfile -NonInteractive -EncodedCommand` (UTF-16LE, base64) or `cmd /d /c`.
- **Who and where:** it runs as the daemon's user, with stdin closed and no sudo, in its own process group (Unix) or Job Object (Windows), which the timeout kills whole.
- **Environment:** on Unix it is clean apart from the owner's login `PATH`; on Windows it is inherited, without the daemon's tokens.
- **Output:**
  - the full output goes to `<home>/logs/owner-actions/<id>.log` (0600). It is unredacted and readable by bots of the same user, so only the redacted tail is ever pushed or commented;
  - `output_tail` is the redacted last 64 KB (home folders under `/Users`, `/home` and `C:\Users` show as `~`);
  - the proposing bot gets a note with the exit code and the tail, and the item or decision gets the same as a comment.
- **Audit:** append-only `owner_action_audit` (proposed, viewed, refused, run, finished, rejected, withdrawn), mirrored to `<home>/logs/owner-actions.log`.

**On a linked computer (R3).** `target_machine` names a linked computer, and `shell` is required, since this computer's default may not run there. The offer and the run go over that computer's peer link:
- **Offer.** The proposing daemon sends peer request `owner_action_offer {id, proposal}`. The target checks it as it would its own bot's proposal (characters, size, shell for its OS, cwd), hashes the pinned files itself, and stores its own copy under the same id, with `origin: "peer:<peer id>"` and its own linked project as the local project. It answers `{action}`. The proposer stores that copy only if every field but the pin hashes is what it offered and the hash matches the fields. Clients see `target_name`.
- **Run, reject, withdraw.** A client on the proposing daemon sends the usual WS requests, which the proposer forwards as `owner_action_run {id, sha256, approved_by}` and `owner_action_close {id, state: rejected|withdrawn, reason?, actor}`.
  - **A run isn't forwarded for now (H-301, owner ruling 6df14f7a).** The proposing daemon refuses it with "Approve on <computer> or your phone". The computer it targets refuses an `owner_action_run` from a peer the same way, and nothing runs. The owner runs it in that computer's app (its own copy, from the offer) or from a phone paired with it. A reject or withdraw still goes over the link, since it only stops the action.
  - The target acts only on an action that peer offered (otherwise `not_found`, audited as refused), by its own copy's hash, through the same single-use claim.
  - A link that is down fails at once with `unavailable` ("… is offline"), and nothing is queued.
- **Progress.** The target sends `owner_action_update {action}` on every change and `owner_action_output {id, chunk}` while it runs. The proposer accepts them only from the computer the action targets, follows its copy and re-pushes both to its own clients.
- **Records.** The target keeps the full log and the audit. The proposer tells its bot how it ended, once, and leaves a comment on its item or decision.

## The project dashboard

`dashboard_get {project_id}` (read) answers with `{type: "dashboard", dashboard}`, which holds what the dashboard's widgets show (H-018 §2.1, H-076, H-102):

- **`needs_you`:** only what the owner must act on (H-112), each row with a `kind`:
  - `release`: the package, with `can_rule`. Its decision gets no row of its own.
  - `decision`: an open decision of the project.
  - `relayed`: one row for every ruling a bot recorded for the owner and they haven't confirmed: `count`, `decision_ids`, `by` (`[{bot_id, count}]`), and `rulings` (`[{id, title, answer, bot_id, at}]`, what the confirm dialog lists). `confirm_relayed {project_id, decision_ids}` (approve) confirms those of `decision_ids` still relayed (ARCH-R42 M1) and answers `{type: "relayed_confirmed", confirmed: [ids], failed: [{id, message}], changed: [ids]}`; `changed` holds the ones answered, reopened or gone since, and the client rereads.
  - `p0`: an open P0 item.
  - `serving_off`: `{title, reason}`, listed first: the served folder is refused, so no build can be published until `[releases] dir` is fixed (H-100). It has no action in the app.
  - With `all_kinds: true` in the request, the kinds the projects home added are listed too (`owner_action`, `permission_prompt`, `bot_waiting`), each as its typed `AttentionRow` in proto3 JSON with `kind` its lowercase name. An older client leaves `all_kinds` out and sees only the kinds above.
- **`wip_overrides`:** moves made over a WIP limit in the last seven days, with their notes. They're the lead's call, so they sit beside Needs you, not in it.
- **Other linked computers (H-178):** the rows are those `attention_rows` lists, so the dashboard and the projects home always agree.
  - This computer's own rows come in the shapes above.
  - Each linked computer's last good part, the board's home included, is added as `{kind: "elsewhere", row, elsewhere}` (only with `all_kinds`). `row` is the typed `AttentionRow` in proto3 JSON with `kind` its lowercase name, and `elsewhere` is the computer to act on it.
  - `needs_you_count` is the projects home's count for the project: this computer's rows plus every linked computer's.
  - When a linked computer isn't answering, its last good rows are still listed, and `needs_you_note` names it ("Can't reach … right now, so this may not be everything that needs you.").
  - `wip_overrides` are this computer's only.
  - **Deprecated** (ARCH-R70): the peer request `dashboard_needs_you {project_id, all_kinds}` (the caller's ids). A daemon no longer sends it.
    - The board's home still answers it for one release, so a linked computer on 0.17.2 or earlier keeps the home's rows. The answer is `{rows, wip_overrides, count}` in the caller's ids.
    - It is removed in 0.18.0 (H-185).
- **`board`:** null without a board. Otherwise:
  - `columns`: each visible column with its `count`, `wip_limit` and `wip_scope`.
  - `blocked` and `stale`: counts of open items.
  - `done_this_week` and `rework_this_week`: since `since`, seven days back. Items brought in by the backlog import never count. Off the board's home, where the history isn't, both are null and `home` names the computer holding the board.
- **`releases`:** the current package and the last two, newest first.
- **`team`:** each bot of the project (`bot`, as in `list_bots`), its items in Doing, and its open task count.
- **`meetings`:** one row per meeting series, `{series, next_at, collecting, last_held}`: `next_at` is when its routine next starts it (null while disabled), `collecting` the meeting taking contributions now, `last_held` the last one closed. Ad-hoc meetings still collecting follow, with `series` null. Meetings are summaries (see below). Empty off the board's home.
- **`action_items`:** the open ones, soonest due first, each with `meeting_name` and `overdue`.

### The projects home (H-128)

The typed surface `hermes.home.v1` (`proto/hermes/home/v1/home.proto`, contract `home` 1, capability `projects_overview`). Each request travels as a binary `HomeRequest` envelope, or as JSON `{type, req_id, ...fields}` answered with the message in proto3 JSON under the proto field names (snake_case; enums by name, timestamps RFC 3339).

- **`projects_overview {project_ids?}`** (read) answers `{type: "projects_overview", overview}`: one `ProjectRow` per live project (all when `project_ids` is empty), ranked pinned first, then `attention.score` desc, `attention.oldest_at` asc, `last_activity_at` desc, name; `rank` numbers that order. It answers at once from local data and each linked peer's last good part: it never asks a peer. `sources` lists each linked computer with its `state` (`OK`, `OFFLINE`, `TIMEOUT`, `OLD_VERSION`; `STATE_UNSPECIFIED` until first asked) and the `as_of` of the data in use; a row whose peer isn't `OK` is `partial` and names it in `stale_sources`. A row is `pinned` when any member computer has pinned it.
  - **Card fields (H-144, additive):** `columns` lists every board column in board order as `{key, name, category, count}` (`category` the lowercase `ColumnCategory` name), and `doing_total` counts the cards in Doing columns while `doing` lists at most 3; both come from the board here, or its mirror off-home, as `doing` does, and are empty without a board. `bots_waiting` counts bots waiting for the owner, summed from each computer's part (`Part.bots_waiting`) like `bots_working`, last good for a peer that isn't `OK`. `current_release.items_ready` counts the release's items in Verify or later (H-137's `ready`), the progress of a planned release.
- **`attention_rows {project_id}`** (read) answers `{type: "attention_rows", attention_rows}`: the rows the project's `attention` counts, this computer's and each linked peer's last good ones, weight desc then oldest first. Row ids are `<kind>:<daemon_id>:<target>` and stay the same across reads; `daemon_id` is the computer to act on it, `project_id` in its ids.
- **`attention_dismiss {id}`** (approve) closes an `owner_question` row (D6); any other kind is refused with `invalid_request`. The bot that asked is told, as a note from the service, that the owner dismissed it without answering (UX-042).
- **`question_comment_id`** (H-211): on an `owner_question` row on a card, the comment that asked, so the owner's reply can name it in `reply_to`.
- **Push `projects_overview_changed {project_ids}`**, debounced at 2 s: a bot's state, a task or message, a card, a decision, an owner action, a permission prompt or a meeting changed one of those rows, or a peer's part changed. Clients refetch.
- **Weights:** release awaiting, owner action, permission prompt 3; P0 item, serving off 2; relayed rulings, bot waiting, off board, owner question 1; a decision 3 when urgent, else 1. Each computer counts only the rows it owns: its prompts, Run cards (`target_machine` here), waiting bots, decisions and serving state, plus releases awaiting the owner and P0 cards when it holds the board.
- **Peers:** `project_attention {project_ids}` (peer request, the callee's ids) answers `ProjectAttention` in proto3 JSON: a `Part` per project linked with the caller, in the caller's ids, with its rows, summary, own working bots, and on the board's home the current release and latest meeting summary. A project not linked with the caller is left out. A peer sends `project_attention_changed {project_ids}` (the receiver's ids), debounced at 2 s, when its part may have changed; the receiver asks again. It also asks on link-up and every 60 s while a client fetched the overview in the last 5 min, at most one request per peer at a time, 3 s each. A peer without `project_attention` is `OLD_VERSION`.
- **Bots** carry `origin {daemon_id, bot_id}`: where the bot runs, a stand-in's peer and remote id, so a client connected to several computers shows each bot once.
- **A card on the owner's message (D5):** `send_user_message` with `item_id` checks the card is on the bot's project's board and open (else `invalid_request`, or `not_found`), stores and delivers the body as `[card <id>] <body>`, and comments "Owner asked <bot>: …" on the card when its board lives here (`commented: false` on a board mirrored from its home).
- **Owner threads (D6, capability `owner_threads`).** A bot's thread is its DM conversation's messages from the owner and its own `message_owner` notes; other bots' messages there aren't in it.
  - **MCP `message_owner {body, asks?, item?}`** (every bot): stores a note from the bot in its thread, nothing delivered. `body` at most 4096 bytes. With `item` the card is commented "To the owner: …" first (through the board's home when mirrored; a card the bot can't comment on refuses the call) and the stored body is `[card <id>] …`. With `asks: true` it opens an `owner_question` row (target `bot`) until the owner writes in that thread or dismisses it.
  - **`item_comment {asks_owner?}`:** after the comment, an `owner_question` row (target `item_id`) until the owner comments on the card (a D5 card message counts), the card closes, or it is dismissed. On a mirrored board the card's `owner_commented_at` and column are seen here. The comment goes to the board's home with `asks_owner: true` (H-211), which keeps the question (`card_question`: the comment and the bot, a stand-in for a linked bot), so the owner's answer reaches the bot wherever it runs; a home from before refuses the argument and the comment goes plainly.
  - **`owner_threads {}`** (read) → `{type: "owner_threads", owner_threads: OwnerThreads}`: one entry per bot of this computer's projects, stand-ins included (asked of their computer, 3 s; one that doesn't answer is listed without `last`), open questions first, then newest message. `bot` is the `BotRef` of its own computer; `project_id` is this computer's.
  - **`owner_thread_get {bot_id, before_num?, limit?}`** (read) → `{type: "owner_thread", owner_thread: OwnerThreadPage}`: messages oldest first (`limit` default 50, max 200), `asks`/`open` on the messages that asked, `last_read_num`, `has_more`.
  - **`owner_thread_read {bot_id, up_to_num}`** (read) → `{type: "owner_thread_marked", owner_thread_marked}`; read state never goes back. `unread` counts the bot's messages after it.
  - `bot_id` may be this computer's id or, for a stand-in, its `BotRef.bot_id`. A stand-in's thread requests go to its computer as peer requests of the same name (in its ids); offline answers `unavailable`.
  - **Push `owner_thread_updated {bot: BotRef, project_id}`** on a bot's note, question, read or dismissal; the bot's computer sends peer notify `owner_thread_updated {bot_id}` to each peer the bot is linked to, which pushes it for its stand-in. Pushes carry no titles or text.
  - Binary arms: `HomeRequest` 5–7, `HomeResponse` 5–7.
  - **Answers posted from the session (H-192, capability `owner_answers`).** When a turn the owner started from chat (`trigger {kind: "owner", via: "chat"}`) ends without a `message_owner` call, its final text item is posted to the bot's thread as the bot's note, once per turn. It is redacted like a prompt summary and cut to 4096 bytes with "… The rest is in Activity.".
    - When: after the `Stop` hook, once the transcript holds the turn's end; a turn that ended more than 2 minutes before the hook is never posted.
    - Not posted: turns started any other way (terminal, a task, a bus message, a routine); turns of a stand-in, which its own computer posts.
    - Each posted turn is stored once (`owner_answer`, keyed by bot and turn). The chat turn then carries `answer_num`, the thread message it became, in `chat` replies and `chat_turns` pushes.
    - A `message_owner` call shows in the transcript as `{type: "sent", to: "owner", msg_kind: "owner", body}`, which is also what tells the capture the turn answered already.
- **Pins (D7, capability `project_pin`).** `project_pin {project_id, pinned}` (control) → `{type: "project_pinned", project_id, pinned}` (binary arm 4). Stored per computer (`project.pinned_at`) and forwarded best-effort as peer request `project_pin` (the callee's id) to each linked computer, which stores it without forwarding. `ProjectAttention.Part.pinned` carries it across. Push `project_pinned {project_id, pinned}`, and the row's `projects_overview_changed`.
- **Redaction (CE-014 F1):** a permission prompt's `summary` and every attention row's `title` mask token-like values (`Bearer …`, `--password …`, `--token …`, `token=`, `password=`, `secret=`, `*_TOKEN=`, `*_KEY=`, `*_SECRET=`, `*_PASSWORD=`) as `***`. A prompt's full `input` is unchanged.

### Flow metrics (B11)

`metrics_get {project_id, range}` (read). `range` is `week` (7 days, the default) or `4w` (28 days). It answers `{type: "metrics", metrics, note}`, computed from the board's history. Items brought in by the backlog import never count.

`metrics` holds:
- **`throughput`:** items that reached Done in the range. `weekly` is the count per week for the last 8 weeks, oldest first.
- **`cycle`:** `{p50, p85, count}` in seconds, from an item's first entry into Doing to Done, for the items done in the range (nearest-rank percentiles).
- **`by_column`:** each in-progress column (Doing through Deploying), with `p50`/`p85`/`count` of the time items spent in it, for stints that ended in the range.
- **`daily`:** one point per day, `{at, wip, columns: {key: count}}`: where every item stood at the end of the day. `wip` counts Doing through Deploying. This is the cumulative-flow series.
- **`reworked`, `past_doing`, `rework_rate`:** items sent back to Doing in the range, over the items that went past Doing in it.
- **`aging`:** the five open in-progress items longest in their current column, `{id, title, column_key, age}`.
- **`expired_tasks`**, **`days`**, and **`since`**.

Without a board, `metrics` is null. Off the board's home, the daemon asks the home (peer request `metrics_get {project_id, days}`) and answers once it has. If the home can't be reached, `metrics` is null and `note` names it.

## Pull requests and checks (H-261)

The typed surface `hermes.pr.v1` (`proto/hermes/pr/v1/pr.proto`, contract `pr` 1, capabilities `pull_requests` and `checks`). It's the same shape as the projects home:
- **Transport:** binary `PrRequest`/`PrResponse`/`PrPush` envelopes (arms 7–9), or JSON `{type, req_id, ...fields}` with the proto field names.
- **Requests:** `pr_list`, `pr_get`, `pr_diff`, `pr_comments` and `review_settings_get` (read). `pr_flag` and `check_rerun` (control). `pr_review_submit`, `pr_comment_add`, `pr_merge_undo`, `review_settings_set` and `cleanup_resolve` (approve; the owner's device or ticket only, never the owner token or a bot). `disk_report` (read).
- **Pushes:** `pr_updated`, `check_updated`, `merge_queue_changed` and `cleanup_updated`.
- **Repositories:** a PR names its `repo` ("owner/name"), because a project's board can span several repositories (gravity and gravity_os). PR numbers stay per project. `pr_list` filters on `states`, a list; an empty list means open and merging.
- **Gate on the capability:** clients show the PR tab only when `hello_ok.capabilities` has `pull_requests`.
- **Bots' MCP tools:** `pr_open`, `pr_push`, `pr_close`, `pr_review`, `pr_comment`, `pr_get`, `pr_list` and `check_report`, defined in `proto/hermes/pr/v1/tools.proto`.
- **Releases cut from main:** they gain `tag`, `commit`, `prs` and `also_included` (`ReleaseFromMain`) at the top level of the release payload. These are new keys, so a client reading today's release is unaffected.
- **Fixtures:** golden fixtures are in `crates/bus/fixtures/pr/`.

**Served to clients (PR-8, H-273).** `hello_ok` advertises `pull_requests`, `checks` and `contracts.pr = 1`.
- **Binary reads:** `pr_list {project_id, states[]}`, `pr_get {project_id, number}`, `pr_diff {project_id, number, from_sha?, to_sha?, path?}` and `pr_comments {project_id, number, sha?}`, as `PrRequest` arms, answered by `PrResponse` (`pr_list`, `pr`, `pr_diff`, `pr_comments`). They need `read`.
  - `pr_list` gives each PR without its comments, and its `mergeable` without the conflict check; `pr_get` gives it in full.
  - `pr_diff` reads the daemon's blob-less cache through SafeGit. `from_sha` defaults to the head's merge-base with main and `to_sha` to the head; any commit the cache holds works, so `from_sha = <approved sha>` gives the delta since an approval. Only hex commit ids are accepted, and a `path` can't start with `:`. The answer has `files` (status, old path, +/−, binary) and `diff`, cut on a whole line at 2 MB with `truncated` set.
  - A comment's `line` is where it is on the shown commit, or the line it was written on when it's `outdated` there.
  - A check's `log_url` is opaque text: `checks/<sha12>-<name>.log` in the project's artifacts on the board's home, or `<computer>:<path>` for a log kept on a linked computer. It's never a path on the reader's disk.
- **`check_rerun {project_id, sha, name}`** (binary, `control`): the owner's re-run, from the app's ticket or a paired device only. The owner token is refused. It answers with the check, queued. Bots use the MCP tool, where the lead and the PR's author may.
- **Other arms** (`pr_review_submit`, `pr_comment_add`, `pr_flag`, `pr_merge_undo`, `review_settings_*`, `pr_comment_resolve`) are JSON requests on this daemon. A binary one is answered `unsupported`.
- **Pushes:** a connection's first PR request for a project starts that project's `PrPush`es (`req_id` 0): `pr_updated {project_id, number}`, `check_updated {project_id, sha, name}` and `merge_queue_changed {project_id}`. They're sent when the board transaction behind them commits, so every change to a PR, its pushes, reviews, comments, checks, flag or merge queue sends one. A check change also sends `pr_updated` for each live PR whose head it counts on.
- **Linked computers (B9):** PRs live on the board's home. A computer that mirrors the board forwards the reads to the home as the peer request `pr_read {project_id, request}` and answers in its own project id. The home relays each change as the peer event `pr_event {project_id, push}`, which is published there in the linked project. While the home can't be reached, a read answers `unavailable`. A linked bot's PR tools go through `board_call` like every board tool.

**Bots' prompts for PRs (PR-10, H-286).**
- **The flag:** a project in the daemon config's `pr_flow_projects` (names, matched without case) has cut over to pull requests (H-279). Its bots' prompts carry a "Code goes through pull requests" section after "Work is on the board", from each bot's next start.
- **What the section says:**
  - `pr_open` with the worktree; `pr_push` after every push;
  - `pr_get` and `pr_comments` for what blocks a merge; `pr_close` to abandon;
  - no review of your own PR; no manual move to Review or Verify;
  - merges only through DevOps' `hermesd pr merge`;
  - owner acts only on the board's home or the owner's phone (rulings 6df14f7a, 1cb9b4df).
- **Rollback:** take the name out of `pr_flow_projects`; the section is gone at the next start.
- **The prompt test** fails on any `pr_…`/`check_…` name in the section that isn't a PR tool the daemon serves.

**Cleanup, the sweep and disk (CL-2, H-275; H-261 §15.4–15.6).**
- **Closed PRs:** `pr_close` queues the PR's cleanup jobs, due 7 days after it closed. They run like a merged PR's: a clean, pushed tree goes, unsaved work is salvaged and the tree held. Rule 1 for a closed PR is that no PR is open on its branch again. Opening a PR on the branch within the window deletes those queued jobs and marks the old branch `kept` ("opened again as PR #n").
- **Closed branches:** the `hermes/pr_merge` gate answers `stale_branches` (closed PRs of the same repository closed 14+ days ago, no live PR on the branch). `hermesd pr merge` deletes each one under a lease on the head it closed at, and sends the results as `stale_branches` in `hermes/pr_merged`. The daemon records only due PRs (`pr_branch_cleanup`). A merged PR's own branch outcome is recorded there too.
- **Daily sweep:** each computer checks hourly and sweeps once a day. It covers the projects that have PRs. Each linked worktree of the project's repositories in the allowed roots is a candidate if its branch has no live PR and it is merged into `origin/main` (and untouched for a day) or idle for 14 days. Candidates go through the same rules (`run::attempt`). A tree in use is skipped until the next day. Orphans are only reported, and `<repo>-rel-*` trees are never touched. A linked computer asks the board's home with the peer request `cleanup_sweep_plan {project_id}` (live branches, trees already held, bots with a live PR), then sends the peer event `cleanup_swept {project_id, found[]}`. The home keeps every row as a `cleanup_job` with no `pr_id`. Salvage older than 30 days is pruned.
- **`cleanup_resolve {project_id, job_id, action: remove|keep}`** (approve; device or ticket on the board's home only). A linked computer answers `forbidden` with "Do it on <home> or your phone.", and the home refuses it inside `pr_owner`.
  - `keep` records the owner's word.
  - `remove` works only on a tree whose work was salvaged. It saves the tree's work again, refuses if any file can't be saved, then runs `git worktree remove --force` under rules 2 and 3. A tree on a linked computer is removed by that computer: the home sends the peer request `cleanup_force`, and the computer answers with the event `cleanup_forced`.
  - That computer refuses unless nothing in the tree changed for 3 days (the newest mtime in it, links not followed), so a forged home can't take a tree in use (ARCH S1). The job stays held with why, and keeps its salvage note so the owner can ask again.
  - It answers `{type: "cleanup_item", cleanup_item}`.
- **`disk_report {machine?, refresh?}`** (read) → `{type: "disk_report", disk_report: {machine, free_bytes, total_bytes, uses[{bot, workspace_bytes, worktree_bytes, cache_bytes, reclaimable_bytes}], as_of}}`.
  - Each daemon takes its own report hourly and keeps it.
  - A linked computer's report is asked with the peer request `disk_report_get`.
- **`cleanup_now {machine?}`** (control) runs the sweep plus the §15.2 cache trims and answers `{type: "cleanup_done", trees, freed_bytes, disk_report}`. On a linked computer it is the peer request `cleanup_now`, accepted only from a board home of a project linked there. It does only what the daily sweep and the cache rules already allow, so it needs no owner proof.
- **Needs you:**
  - `CLEANUP_HELD` (14), target `cleanup_job_id`: a job held 3+ days or failed, with no owner word yet. The board's home builds it. Its `cleanup` (`CleanupRow`: `state`, `machine`, `bot`, `pr_number`, `uncommitted`, `unpushed`, `salvaged`, `reason`, `since`) is what clients word the row from (UX-055); `title` is a plain-words fallback with no path. Offer Remove anyway only when `salvaged`.
  - `DISK_LOW` (15), target `machine`: this computer's last report is under 20 GB free. Each computer builds its own.
- **On the PR:** the `pr_get` JSON keeps `cleanup` (the jobs) and adds `cleanup_summary {state, freed_bytes, items, branch_deleted}`, which maps to `PullRequest.cleanup`. Every job change pushes `cleanup_updated {project_id, number}` with `pr_updated`.

**PRs from linked computers (PR-9, H-285; §1, §4.3, §12.9).**
- **Bots:**
  - `pr_open`, `pr_push`, `pr_review`, `pr_comment`, `pr_comment_resolve`, `pr_flag` (lead), `pr_get` and `pr_list` run on the home as the bot's stand-in, through `board_call`. Every record (author, pusher, reviewer, comment author) names that stand-in.
  - **A worktree** passed to `pr_open` or `pr_push` is checked on the bot's own computer (`worktree::inspect`): it exists there, it's in a folder that bot may work in, and it's a git worktree. The path is then taken out of the call's args and sent beside the `board_call` frame as `worktree {path, main_clone, origin, branch}`.
  - **The home** checks that worktree's origin and branch against the PR's repository and branch. It records the worktree under the linked computer's name (the home's name for that peer), for the stand-in.
- **The owner, in a linked computer's app (H-285 must-fix, owner ruling 6df14f7a):** the owner's PR acts are **not** taken from a linked computer.
  - **Why:** a peer token is a file bots can read, and it works at both ends of a link. A bot could dial the home as that computer and claim the owner approved there.
  - **Refused on the linked computer**, before anything is sent, with "Approve on <home> or your phone": `pr_review_submit`, `pr_comment_add`, `pr_comment_resolve`, `pr_flag`, `pr_merge_undo`, `release_leave_out`, `review_settings_set` and the binary `check_rerun`.
  - **Refused on the home too:** a `pr_owner` frame carrying any of these is refused with the same words, whatever its `via`, and `owner::provenance` refuses every Peer proof. Only `review_settings_get` (a read) is served.
  - **For later:** the `via` plumbing stays, and `peer::owner_trust::trusted` is where signed approvals would switch it on. It reads `Config.trust_forwarded_owner_acts`, which is never read from a config file (`serde(skip)`) and is `pub(crate)`: it's off in every daemon, and only tests of the forwarded path set it, through `Config::trust_forwarded_owner_acts_for_tests` (built only under `cfg(test)` or the crate's `test-support` feature, which only its dev-dependency turns on).

**The owner's hands over a peer link (H-303, owner ruling 1cb9b4df).** For the same reason, what a linked computer says its owner did there isn't taken, whatever the frame's `owner_verified` says. Watching a linked bot's terminal or browser is unchanged, and so are this computer's own owner paths (device or ticket).
- **Typing into a bot's terminal**, a permission prompt's answer included (it is typing), and a forced resize: the linked computer refuses `input` for a linked bot with "Do it on <computer> or your phone…" and sends nothing; a forced resize goes as unforced. The bot's computer drops a peer's `term_input` and forced `term_resize`. A plain resize still goes.
- **Browser control:** the linked computer refuses `browser_input` for a linked bot the same way; the bot's computer drops a peer's `browser_input`.
- **The owner's chat:** the linked computer sends no `owner_verified`, and the bot's computer records no owner proof for a peer's chat, so it's never typed into a composer (H-195 D2, H-209). It is delivered as a plain bus message, shown to the bot as from `UNVERIFIED @ <computer>` (`[msg #N from UNVERIFIED @ MAC · chat] …`; `check_inbox` says `"from": "unverified @ <computer>"`), never as `USER`.
  - **Clients (H-306)** are sent it the same way: the owner thread's `ThreadMessage` and `MessageBrief` carry `unverified_from: <computer>` with `from_owner: false`, and `message_new`, `list_messages` and `search` messages carry `unverified_from`. The app shows it as "From <computer>, unverified".
  - **It answers nothing:** it closes no question the bot asked the owner, in the thread or on a card (H-211), and a turn it starts isn't posted to the owner thread as an answer (H-192).
- **The app:** a linked bot's terminal says "Do it on <computer> or your phone…" above it and sends no typing; its browser offers no Take control and says to do it on that computer or a phone. A linked bot has no permission cards here: its prompts show in its terminal, and are answered on its computer.
- **For later:** each gate is behind `peer::owner_trust::trusted`, with the old `owner_verified` checks kept under it for signed approvals.
- **Checks on a linked computer** are H-283's peer `check_run`/`check_result`, accepted on the home and bound to the job's sha.

**Bots' PR tools (PR-1, H-266).** Every bot on a project with a board gets these; the checks below decide who may act. All of them run on the board's home computer (`pr`, `pr_push`, `pr_worktree`, migration `PR_CORE`).
- **`pr_open {item, branch, worktree?, title?, body?, repo?}`:**
  - Only the card's assignee, or a bot tasked on the card, may open it. The card must be in Doing (or already in Review), with no other open PR.
  - `repo` must be the project's repository or one the owner added. Anything else is refused with "<name> isn't one of this project's repositories", and nothing is fetched.
    - The owner adds them with the WS request `set_project_extra_repos {project_id, urls}` (approve), from the app's ticket or a paired device only. The owner token is refused and no bot tool sets it.
    - It answers `{type: "project", project}`, with `extra_repos`.
  - The daemon fetches the repository into its own cache, `<home>/cache/repos`, and reads the branch's tip there: that tip is the head.
  - It records PR #n, numbered per project. `head_patch_id` is `git patch-id --stable` of the diff from the head's merge-base with main to the head.
  - It links the branch and `#n` on the card, and moves the card Doing → Review. A linked PR counts as the change note.
- **`pr_push {number, sha, worktree?}`:**
  - Accepted only when the fetched tip of the branch equals `sha`; the reporter is recorded as a pusher.
  - A rebase onto main without conflicts keeps the patch-id; a new commit changes it.
- **Pushes nobody reported:**
  - The daemon notices them before `pr_get` and `pr_list`, and every 5 minutes.
  - It sets `moved_unreported`, records the tip once with no pusher, and leaves the head alone.
  - The next `pr_push` of that tip clears it.
- **`worktree`:**
  - It must be a git worktree whose `origin` is the PR's repository, on the PR's branch.
  - It must sit inside the bot's own workspace or one of its `<repo>-wt-<slug>[-…]` folders in the trusted paths, resolved with links followed.
  - Otherwise the call is refused and nothing is recorded.
- **`pr_close {number, reason}`:** only the author or the lead closes a PR. It is closed unmerged, and its card goes back to Doing.
- **`pr_get {number}` and `pr_list {states?}`:** a PR with its pushes and worktrees; empty `states` means open and merging.
- **Off the board's home:** a call is forwarded there like any board tool, and a worktree path from another computer is refused (PR-9 handles that).

**Reviews (PR-3a, H-268).** These use migration `REVIEWS`, which adds the tables `review`, `review_comment`, `pr_need` and `pr_review_task`.
- **Required roles:**
  - After every head, the daemon reads `.hermes/reviewers.toml` at the PR's base, from its own cache.
  - The changed paths of `merge-base..head` give the required roles. They are listed as `required_roles` on `pr_get`.
  - Each role is filled by a board role: `architect` → reviewer.arch, `ux` → reviewer.ux, `ce` → reviewer.ce (new, `ROLE_REVIEWER_CE`), `devops` → devops, `qa` → tester.
- **`pr_review {number, sha, role, verdict, summary?, findings[], artifact?}`:**
  - The bot must hold the role's board role, and `sha` must be the head.
  - It is refused while the PR is `moved_unreported`; the pusher reports the tip first.
  - `changes_requested` needs at least one `must` finding.
  - The `owner` role is refused over MCP.
  - The verdict is stored with the head's patch-id. `stale` is computed as the review's patch-id ≠ the PR's head patch-id, so an update with main keeps it and a fix-up or a conflict resolution doesn't.
- **Who may not review it (§4.4):**
  - the PR's author;
  - the card's assignee;
  - any bot that ever reported a push to it;
  - a bot whose worker (`created_by_bot_id`) did;
  - for architect, ux and ce, a bot with the dev role tasked on the card.
- **Who seats reviewers:**
  - `reviewer.ce` is the owner's to give, from a paired device or the app's ticket. The owner token and the lead are refused.
  - The lead may give reviewer.arch and reviewer.ux to other bots, never to itself.
- **Review tasks:**
  - When a PR opens or its head changes, each bot role with no fresh approval and no open review task gets one task from the daemon (`from_bot_id` empty), linked to the card.
  - It goes to the first holder §4.4 allows. A role nobody may fill gets none, and the daemon logs it.
  - The review closes it.
- **`pr_follow_up {number, review_id, finding}`:**
  - The reviewer, the PR's author or the lead files a `should` or `nit` finding as an Inbox chore, labelled `follow-up`, related to the PR's card.
  - The finding records `follow_up_item_id`, so it can't be filed twice.

**The owner as a reviewer (PR-4, H-269; ruling 7629a873).** It uses migration `REVIEW_SETTINGS`, which adds the tables `project_review_settings` and `pr_shape`.
- **The setting:**
  - `review_settings_get {project_id}` (read) answers `{type: "review_settings", review_settings: {owner_review, owner_review_areas, areas}}`. `areas` are the `reviewers.toml` area names on main.
  - `review_settings_set {project_id, owner_review: all|areas|flagged|none, owner_review_areas?}` (approve) is accepted only from a paired device or the app's ticket. The owner token is refused and nothing is stored.
  - No row means `all`.
- **`owner_review_required`, shown on `pr_get`:**
  - A PR on the policy files (`.hermes/*.toml`): always, whatever the setting, `none` included (H-313). Migration `PR_SHAPE_POLICY` adds `pr_shape.policy`; the merge gate reads it again.
  - `none`: otherwise never.
  - Otherwise, always for security work: a matched area whose roles include `ce`, or the policy files.
  - Then by setting: `all` → yes; `areas` → when an area the PR matches is listed; `flagged` → when the PR is `owner_flagged`.
  - Docs-only PRs follow the setting.
- **Needs you:** a row `PR_REVIEW` (target `pr_number`) on the board's home appears only once every required bot role has a fresh approval. It leaves when the owner approves the current change.
- **`pr_review_submit {project_id, number, sha, verdict, summary?, findings?}`** (approve; device or ticket only):
  - records the owner's verdict on the head, with provenance `device:<id>` or `ticket`;
  - is refused while the branch moved unreported;
  - `changes_requested` needs a `summary`, and sends the card back to Doing.
  - Answers `{type: "pr", pr, review}`.
- **`pr_comment_add {project_id, number, sha, path, line, side?, body, severity?, reply_to?}`** (approve; device or ticket only) stores an owner line comment on a commit of the PR. Bots' comments and re-anchoring come in H-282.
- **`pr_flag`:**
  - The owner uses WS `pr_flag {project_id, number, flagged, reason}` (approve, from a device or the app's ticket only; the owner token is refused).
  - The lead uses the MCP `pr_flag` tool, which can only set a flag.
  - Flagging needs a reason, and `pr_flag` records who flagged.
  - Only the owner clears a flag, so under `flagged` no bot can take the owner's review away. The lead's flag never replaces the owner's.
  - The PR shows `owner_flagged` and `owner_flag_reason`.

**Line comments (PR-3b, H-282; §1.4, §5.1(3)).** They're stored in `review_comment`, from H-268; no migration.
- **`pr_comment {number, sha?, path?, line?, side?, body, severity?, reply_to?}`** (MCP, any bot in the project):
  - It comments on a line at a commit the PR has had; `side` is `new` (default) or `old`.
  - The anchor must name a line of a text file at that commit (`old` side: at its merge-base with the PR's base). A missing path, a folder, a binary file or a line past the end is refused.
  - A reply (`reply_to`) joins the thread of the comment it answers, takes the thread's anchor, and carries no severity.
- **`pr_comments {number, sha?}`** (MCP) lists every comment as shown on `sha` (default: the head):
  - each one has `shown_line`, where its line went, through the zero-context diff of its file from the commit it was written on;
  - it's `outdated`, with no line, when that line was changed or removed, when the file is gone or isn't a file, or when it changed without line hunks (binary or `-diff`);
  - a comment on the `old` side is outdated on any other commit;
  - it's never shown on a line it wasn't written about, and `line`/`sha` keep the original anchor.
  - `pr_get` includes `comments` as shown on the head.
- **The diff:** it runs through SafeGit (H-289), so no external diff or textconv runs.
- **`pr_comment_resolve {number, comment_id}`:**
  - It resolves a thread; resolving a reply resolves its thread.
  - A `must` thread: only its author or the owner.
  - A thread the owner started: only the owner.
  - Any other thread: its author or the PR's author, over MCP.
  - The owner resolves any thread over WS, from a device or the app's ticket.
- **An open `must` thread** (a thread's first comment with `severity: must`, unresolved) is a mergeable blocker: `unresolved_must`, subject `comments`.

**Merging (PR-6a, H-271; §5.1, §5.2, §16).** Migration `MERGE_QUEUE` adds the tables `pr_merge` and `review_withdrawn`.
- **`pr_get` shows `mergeable {ok, blockers[]}`.** Each blocker is `{kind, text, subject, paths}`, where `kind` is a lowercase `BlockerKind` (§5.1):
  - **open and reported:** `not_open`, `moved_unreported`;
  - **every required role approved this change:** `review_missing`, `changes_requested`;
  - **the owner, if needed:** `owner_review`;
  - **no open `must`:** `unresolved_must`;
  - **every required check passed on the head or its tree:** `check_pending`, `check_failed`;
  - **the head descends from main's tip:** `behind_main`, or `conflicts` with `paths` from `git merge-tree`;
  - **the card isn't blocked and its ACs other than post-install ones are ticked:** `card_blocked`, `ac_unticked`.
- **The queue (nobody presses merge):**
  - every 2 s the daemon puts each newly mergeable PR at the back of its project's queue, and takes out any that stopped being mergeable;
  - the head of the queue gets a 10 s `merging` window (`pr.state = merging`, `merge.merge_at`) in which nothing is pushed;
  - once the window ends, DevOps gets a `pr_merge` daemon task, linked to the card, naming `hermesd pr merge <n>` (`merge.state = handed`; H-284 runs it);
  - one PR at a time per project: after a merge, main moved, so the next one shows `behind_main` until its author updates it, and is out of the queue meanwhile. Updated without conflict, its approvals carry (same patch-id) and its checks are queued again on the new head.
- **Undo:**
  - `pr_merge_undo {project_id, number}` (approve; from a device or the app's ticket only) works only in the window.
  - It returns the PR to `open` and withdraws the owner's approval of this change (kept in `review_withdrawn`, no longer counted).
  - The PR stays out of the queue until its change or the owner's approval changes. After the hand-off it is refused.

**The merge executor (PR-6b, H-284; §5.2, §5.3, §6.5, §15.2).** Migrations `PERMISSION_EXTRAS_PR_MERGE` (the `pr_merge` extra) and `PR_MERGE` (one live PR per card **per repo**; the `pr_merge_stuck` table).
- **`hermesd pr merge <n> [--dry-run]`**, run by DevOps in its own checkout of the PR's repo on its `pr_merge` task. It asks the daemon over the local endpoint, like `release land`:
  - **`hermes/pr_merge {number}`:** the bot holds the `pr_merge` extra (off for every bot until the owner grants it), is the project's DevOps and holds the PR's open `pr_merge` task. The daemon fetches the repo; the branch must still be at the PR's head. It recomputes the roles and shape the change needs **at the merge** (main's tip to the head; the base's `reviewers.toml` at main's tip), then every §5.1 condition. A PR that no longer holds leaves the queue, its task is cancelled, and the reason comes back. Otherwise it answers `{number, head_sha, branch, main_sha, repo, repo_url}`.
  - **In the checkout** (git hooks off): origin must be that repo; the branch on origin must be at the head and main an ancestor of it. Then `git push origin <head>:refs/heads/main`, never forced, and `ls-remote` of the repo must show main at the head.
  - **The branch** is deleted with `--force-with-lease=refs/heads/<branch>:<head>`, so it goes only while its tip is still the merged commit; a branch that moved is kept and reported.
  - **`hermes/pr_merged {number, sha, branch: {deleted, note}}`:** the daemon's own fetch must show main at `sha`, the PR's head, and the gate must have passed this head (`pr_merge_check`: same head, the main it saw an ancestor of it, under 15 min old). Otherwise main moved outside the gate: nothing is recorded, the card stays, and the owner's Needs you shows a `MAIN_MOVED_OUTSIDE` row (weight 3). The PR becomes `merged` (`merged_sha`, `merged_at`, `merged_by`), leaves the queue, and its task is done. A kept branch is said on the card. Answers `{number, sha, card, moved, waiting_for[], branch_kept}`.
- **Board:** once every PR of the card, in every repo, is merged, the daemon moves the card Review → Verify ("PR #n merged"). A manual Review → Verify on a card with a PR link is refused (`review.merged_by_pr`); spikes and chores without a PR keep the H-154 path.
- **Guard:** the `pr_merge` extra pre-approves exactly `<this daemon's binary> pr merge`, run as given (no `--config` or home override). It doesn't lift the main denies: a raw `git push … main` stays refused.
- **Stuck merges:** a merge `handed` for 30 min goes to DevOps again (the old task expires) and is recorded in `pr_merge_stuck`. Until it merges or leaves the queue, the owner's Needs you shows a `PR_MERGE_STUCK` row (weight 2): "PR #n (card) is waiting for DevOps to merge it; asked N times".

**Releases cut from main (PR-7, H-272; §6.1–6.4, UX-051 decision 10).** Migration `RELEASE_PR` adds `release_cut`, `release_pr`, `release_leave_out` and `pr_reverted`. A release is a commit on main: there are no release branches.
- **`release_cut {version, commit?, release_id?, repo?}`** (MCP, DevOps):
  - The commit defaults to main's tip. It must be on main (an ancestor of the tip, in the daemon's own fetch); any other commit is refused.
  - **The range** runs from the commit of the latest release tagged before it (`previous_commit`) to the cut commit.
  - **Contents:** the PRs merged in that range are the release's `prs`, and their cards are its items. A card whose PRs there were all reverted by a Leave out is not an item.
  - **From a planned package** (`release_id`): planned cards with no merged PR show in `not_merged`, and merged PRs whose cards nobody planned are in `also_included`.
  - The package is then `assembling`, and the gate is unchanged. Every build must name the cut commit as its `source_commit`.
- **The release JSON gains** `tag` (empty until pushed), `tag_name` (`desktop-v<version>`), `commit`, `previous_commit`, `repo`, `prs`, `also_included` (`PrRef`s, plus `reverted` and `revert`) and `not_merged`. `pr_get` gains `reverted_by`.
- **`hermesd release tag <release> [--dry-run]`** (DevOps, the `release_main` extra, run in its checkout):
  - **The gate, `hermes/release_tag`:** the release was cut from main, the owner approved it, and its builds come from the cut commit.
  - **In the checkout:** the commit must be on origin's main. The annotated `desktop-v<version>` is pushed, and nothing else; an existing tag must already be on that commit. `ls-remote` checks the result.
  - **The record, `hermes/release_tagged`:** the daemon asks the repository itself before recording it.
  - `release land` refuses a release cut from main.
- **Leave out, `release_leave_out {project_id, release_id, prs}`** (WS approve; the owner's device or ticket only). It's possible before the tag, while the release is assembling, built, awaiting the owner or held.
  - **The package starts over:** its items go back to Verify, the builds and tests are dropped, and the ruling is withdrawn.
  - **Left-out PRs that merged after every kept one:** the release is cut again at the last kept merge (mode `recut`). Their cards stay in Verify, with a comment that they ship later.
  - **Otherwise** (mode `revert`):
    - A later kept PR that changed the same files is refused by name: "PR #n (card) changed <files> after #m".
    - Otherwise DevOps gets a task to run **`hermesd release leave-out <id>`**. In a fresh worktree of origin's main it runs `git revert --no-edit <base>..<merged>` for each PR, newest first, pushes `leave-out/<version>-<id>`, and asks `hermes/leave_out_pushed`.
    - **The daemon checks the branch first:** `hermes/leave_out` records `main_at`, main as it was then, and the command reverts from exactly that commit. At `hermes/leave_out_pushed`, the daemon computes the expected tree itself, chaining `git merge-tree --write-tree --merge-base=<merged> <tree> <base>` over the ranges on `main_at`'s tree. The head must descend from `main_at` and hold exactly that tree; otherwise nothing is opened.
    - **Then** it opens the branch as the owner's PR (author `owner:<device or ticket>`): no bot review, no owner review, checks required, the normal queue and `hermesd pr merge`.
    - **`hermesd pr merge` checks it again:** for an owner-authored PR, the head and tree must still be the ones recorded.
    - **A later `pr_push`** to it makes the pusher its author, so the usual reviews apply.
    - **When it merges:** the left-out PRs get `reverted_by`, the release is cut again at the new main, and their cards go back to Doing ("left out of <version>").
  - **Re-adding** a left-out change later is a new PR that reverts the revert.
- The `release_main` extra pre-approves `release land`, `release tag` and `release leave-out`, run as given. It **no longer lifts the main denies** (CE S1 on H-284): no bot, DevOps included, pushes or merges to main itself. Main moves only through `hermesd pr merge` and `release land`, which push from their own process.
- **Process rule (not code):** DevOps proposes at most one release a day.

**Checks (PR-5a, H-270).**
- **Which checks:** each reported head queues the required checks of the base's `.hermes/checks.toml`, filtered by the paths the PR changes. A head's own `checks.toml` changes nothing.
- **A broken base file:** if the base's `checks.toml` doesn't parse, a single `.hermes/checks.toml` check is recorded as `error`, so nothing counts as checked.
- **Repairing it:** a PR that changes `checks.toml` to a file that parses gets the head's checks plus a passing `.hermes/checks.toml` check, so the repair can merge. Like any policy change, it still needs architect, ce and the owner.

**Running checks (PR-5b, H-283).**
- **Tools:** each daemon probes `node`, `pnpm`, `cargo`, `xcodebuild`, `python3`, `mkdocs` and `docker` at boot and hourly (`checks.probe_interval_secs`), plus `os` and `disk_free_gb`. It keeps them in `machine_tool` under its own name and sends them to each linked computer as the peer event `machine_tools {tools}`, also when a link comes up.
- **Routing:** the board's home routes each queued check to an online computer whose tools cover its `needs` and the program its `run` starts with, matching `machine` by name or OS (`windows`, `mac`). The least busy wins, this computer first on a tie. A check with no such computer keeps waiting, with the reason in its `note`.
- **The runner:** no bot and no AI worker. The daemon of the chosen computer runs the check itself, as the identity `daemon:check-runner` (not a bot: no inbox, no parent, never a PR's author or pusher). It clones the repository at the exact sha into `<home>/run/checks/<job>/check` (built under `<home>/run/git`, then moved in), then starts `hermesd check run <job folder>` with a clean environment (tool paths and locale only, no tokens). That runs the job's `run` from the base's `checks.toml` in the checkout, all output to `check.log`, and exits 0 when it exited 0, 1 when it didn't, 2 when it couldn't start. The result is that exit status: `pass`, `fail` or `error`. The checkout is removed once it has run; the log is published under the project's artifacts, or, run on a linked computer, stays there as `<computer>:<path>`.
- **Linked computers:** the home asks with the peer request `check_run {project_id, job, url, sha, run}`. The computer clones from its own URL for that repository (it refuses one the linked project doesn't have, and refuses `at_capacity` under its disk floor, so the check waits) and answers at once. When the run ends it sends the event `check_result {job, result, note, log}`; the home accepts it only from the computer the job was routed to.
- **`check_report` (MCP):** a bot a check was dispatched to may report `running` or `error` only. A `pass` or a `fail` over MCP is refused: they come only from the exit status.
- **Load:** at most `checks.jobs_per_machine` (default 2) open check jobs per computer, every project's. No job starts on a computer with less than `checks.disk_floor_gb` (default 20) GB free; it waits instead.
- **Re-runs:** `check_rerun {sha, name}` (MCP) for the lead or the PR's author, and for the owner over the `hermes.pr.v1` `check_rerun` request (H-273). An `error` is retried once automatically, including a job that ended without a result (the daemon restarted while it ran, or a linked computer sent nothing within 7 hours); a `fail` never is.

## Meetings

Meetings live with the board, on its home (H-017 §1.5, H-020 §4, H-102). A **series** (`standup`, `refinement`, `demo`, `retro` or `adhoc`) owns a routine: creating or changing one upserts the routine with the facilitator as its bot, the series' cron and time zone, and a prompt telling it to run the meeting. A new facilitator gets a new routine; disabling the series disables it. Each run, the facilitator calls `meeting_start`. That opens an occurrence (`MTG-<date>-<type>`, collecting), freezes `inputs_snapshot` (the board's columns with counts, Doing, blocked and stale items, the open action items) and sends each attendee bot one note. Attendees `meeting_contribute`, the facilitator (or the lead) records `action_add`s and `meeting_close`s it as held, with outputs by section and a summary of at most ten lines, or skipped with a reason. Open action items carry over: `meeting_get` lists those of the series' earlier meetings as `carried_over`. `action_promote` (lead) turns one into a chore in the board's Inbox, linked to its meeting (link kind `meeting`), and sets the action's `item_id`. Attendees and action owners are bot ids, or `owner`.

**MCP tools.** Every bot: `meeting_list {type?, upcoming?}`, `meeting_get {meeting_id}`, `meeting_start {series_id}` (its facilitator or the lead; ad-hoc with `name`, `attendees` for the lead), `meeting_contribute {meeting_id, section, body, item_refs}` (attendees and the facilitator), `meeting_close {meeting_id, outputs, summary?, skip_reason?}` and `action_add {meeting_id, text, owner, due_at?}` (facilitator or lead), `action_update {action_id, status?, text?, due_at?}` (the action's owner, the facilitator or the lead). Lead: `meeting_series_upsert {series_id?, type, name, cron, tz, facilitator, attendees, input_scope?, enabled?}` and `action_promote {action_id, title?}`. The service checks who may act, so a facilitator needs no board role.

**Series limits (ARCH-R48).**
- **Name:** at most 80 characters, with no line breaks or quotes, because it is quoted in the routine's prompt. Upsert refuses anything else.
- **Prompt:** it quotes the name, cleaned and cut to 80 characters again however it was stored, and says who set the series up ("set up by Team Lead", or "the owner").
- **Cron:** it may fire at most once an hour. One that fires more often (every minute, every 30 minutes, 9:00 and 9:30) is refused.

**From a linked computer.** The meeting tools are board tools. A bot whose board lives on a peer reaches them through `board_call` (B9) and acts on the home as its stand-in. The home's roles and attendee lists decide what it may do. While the home is unreachable, they're refused as other board writes are.

**WebSocket (owner).** Fields as the MCP tools, plus `project_id`, except a meeting's type, sent as `meeting_type` because `type` names the request:
- read: `meeting_list` → `{type: "meetings", series, meetings}`; `meeting_get` → `{type: "meeting", meeting}`.
- control: `meeting_series_upsert` → `{type: "meeting_series", series}`; `meeting_contribute` (the owner as an attendee) → `{type: "meeting", meeting}`; `action_update` and `action_promote` → `{type: "meeting_action", action}`.
- Off the board's home they're refused with the computer to use.
- Every change pushes `{type: "meeting_event", project_id, meeting_id?}`; clients reread.
