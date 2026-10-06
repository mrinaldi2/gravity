# Work guardrails

Owner ruling 0cdec563: no work without a task on the board. H-125 and H-135
build it; spec `H-123`, review ARCH-R57 and ARCH-R59.

| # | Rule | Where |
|---|---|---|
| G1 | Every task and spawn names a board card. A nested task inherits its parent's card. | `mcp/task_card.rs` |
| G2 | A root task is budgeted per card, under a ceiling across the project. | `mcp/task_card.rs`, `config/task_limits.rs` |
| G3 | A note from a bot is at most 800 bytes (`bus::MAX_NOTE_BYTES`). Work goes out as a task, and long content goes in an artifact. | `mcp/tools.rs` |
| G4 | A bot working for 10 minutes with no open task on a card gets a Needs you row, a "Working off-board" badge, and one note to the lead per episode. | `offboard.rs` |
| G5 | A routine names a card (`item`) when its project has a board. Routines without one are listed in Needs you. | `mcp/routines.rs` |

## Notes (G3)

The daemon refuses a bot's note over the cap and says to send a task. The
cap is enforced only where the note is sent, so these are exempt:

- the daemon's own notices;
- the owner's messages;
- notes delivered from a linked computer.

The prompt tells every bot that a note never authorises work.

## Off-board work (G4)

Each daemon checks the bots it runs every 30 seconds. A bot is off-board
when all three hold:

- it is working, or waiting for approval;
- it holds no open task linked to a card;
- its current turn counts.

A turn counts unless one of these holds:

- **The owner started it.** Under owner ruling 06ac8d95, a conversation
  needs no card. That changes once the turn becomes work: it changes files,
  runs a build, delegates a task, spawns a worker or hands back an artifact.
- **It is a run of a routine that names a card.**

After 10 minutes off-board, the bot is flagged:

- Its view gets `off_board_since`, which drives the "Working off-board" tag
  on its Team card.
- The project gets an `off_board` attention row (`crate::attention`): it is
  in the dashboard's Needs you and counts on the projects home. Each
  computer reports the rows of the bots it runs.
- The project's lead gets one note.

If the lead is the bot that went off-board, only the row is raised. The
episode ends when the bot goes idle or takes a task with a card, and the
next episode is flagged afresh.

The rows are this computer's own. A linked computer shows its own bots,
not this one's.

## Task limits are per computer

`[tasks]` is read from each computer's own `hermesd.toml`, and each daemon
enforces it on the bots it runs:

```toml
[tasks]
per_card = 3     # open root tasks one bot may hold on one card
root_lead = 12   # open root tasks the lead may hold across the project
root = 3         # the same, for every other bot

[tasks.projects.gravity]   # by project name; omitted fields inherit
per_card = 4
```

The limits are not synced across linked computers. To keep one budget for
the whole team, set the same values on every computer. Each bot's prompt
states the numbers its own computer enforces. The daemon reads the file
when it starts and rewrites every bot's prompt then, so a change takes effect,
in the enforcement and in the prompts, after a daemon restart.

## The board's home across restarts

A computer that doesn't hold a linked project's board remembers which peer
does, in the `board_home` table. So after a restart, before the home has
been heard from again, the project still counts as having a board, and its
tasks and routines still need a card. The record is dropped when the home
says it no longer holds the board, or when the project is unlinked.
