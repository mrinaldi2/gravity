# Fixture — Backlog

A made-up backlog in the format of artifacts/backlog.md, for the B6 import tests.
States: Inbox → Ready → Doing → Review → Verify → Awaiting owner → Deploying → Done.

| ID | Title | Why | Platform | Size | State | Owner / task | Decision |
|---|---|---|---|---|---|---|---|
| H-001 | Rename plan: Old → New | Owner ruling | all | M | Done — plan approved | Architect | aaaa1111, bbbb2222 |
| H-002 | Safe delete dialog — desktop | Enter deletes | desktop | S | Review (abc1234, rework done) | Desktop Dev | |
| H-003 | Safe delete dialog — iOS | Same on iOS | iOS | S | Verify (QA testing) | iOS QA · 1234abcd | |
| H-004 | Set up routines | Run the process | process | S | Dropped — folded into H-007 | Scrum Master | |
| H-005 | Version negotiation | Exact match breaks phones | desktop+iOS | S-M | Inbox | | |
| H-006 | BUG: mirror freezes after restart | Remote terminals freeze | desktop | S | Doing (worker diag) | | |
| H-007 | Board design | See H-007-design.md | desktop (+iOS) | M | Done (H-007-design.md + mockups.html) | UX Designer | cccc3333 |
| H-009 | Push notifications | Approvals wait | iOS+infra | L | Inbox — needs owner decision | | |
| R1-I2 | Rename: iOS project | Rename release 1 | iOS | S | Review (branch R1-I2 c5dcef4) | iOS Dev | dddd4444 |
| R1-D1..D3, R1-I1 | Rename release 1: the rest | | desktop/iOS | S-M | Ready | | |
| B1 | Board: contract scaffold | Critical path | desktop | M | Changes needed (M1 doc) | Desktop Dev | |
| B3 | Board guards | Critical path | desktop | M | Approved w/ M1 | Desktop Dev | |
| G2+G3 | Usage windows | Budget | desktop | S | Done | worker | |
| REL-D-1 | First desktop release | release/desktop-0.14.0 | desktop | — | Approved → rolling out Mac → imac | DevOps | eeee5555 |
| I0′ | Rework I0 to protos | ADR | iOS | S | ON HOLD (owner paused iOS) | iOS Dev | |
| U2 | Board read-only | Top priority | desktop | M | Next | | |
| H-011 | Plan: protobuf + TLS | Owner direction | all | M | Awaiting owner | Architect | |

Note: this file is a stopgap.

### H-040 — Windows: name the holders of the home
- Source: the install failed at probe_move.
- Do: list the holders with the Restart Manager API.
