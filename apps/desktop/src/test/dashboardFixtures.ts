// The project dashboard for tests and stories (H-076, H-018 §2.1).

import type { BoardSummary, Dashboard, NeedsYou } from "../protocol/dashboard";
import type { Bot } from "../protocol/entities";
import { bot } from "./fixtures";
import { ACTION_ITEMS, MEETING_ROWS } from "./meetingFixtures";
import { deployment, release } from "./releaseFixtures";

const DEV = bot({ id: "dd", name: "Desktop Dev", state: "working", avatar: "icon:moss" });
const IOS = bot({ id: "ios", name: "iOS Dev", state: "working" });
const TESTER = bot({
  id: "tw",
  name: "Tester Win",
  state: "waiting_for_user",
  state_reason: "which build?",
});
const ARCH = bot({ id: "arch", name: "Architect", state: "ready" });
const WORKER = bot({ id: "w1", name: "worker-1", state: "working", temporary: true });

export const DASH_BOTS: readonly Bot[] = [DEV, IOS, TESTER, ARCH, WORKER];

function column(
  key: string,
  name: string,
  category: string,
  count: number,
  wip_limit: number | null,
  wip_scope: "column" | "per_assignee" = "column",
): BoardSummary["columns"][number] {
  return { key, name, category, count, wip_limit, wip_scope };
}

const BOARD: BoardSummary = {
  columns: [
    column("inbox", "Inbox", "inbox", 9, null),
    column("ready", "Ready", "ready", 6, null),
    column("doing", "Doing", "doing", 3, 1, "per_assignee"),
    column("review", "Review", "review", 2, 3),
    column("verify", "Verify", "verify", 3, 3),
    column("approval", "Awaiting owner", "approval", 4, 5),
    column("deploying", "Deploying", "deploying", 0, null),
    column("done", "Done", "done", 41, null),
  ],
  blocked: 2,
  stale: 3,
  done_this_week: 7,
  rework_this_week: 1,
};

/** A busy project: a release, a decision, a P0 and an override need you. */
export function dashboard(over: Partial<Dashboard> = {}): Dashboard {
  const waiting = release();
  return {
    project_id: "p1",
    as_of: "2026-10-05T10:42:00Z",
    since: "2026-09-28T10:42:00Z",
    home: null,
    needs_you: [
      { kind: "release", release: waiting },
      {
        kind: "decision",
        id: "dec-1",
        title: "Push notifications: pay for an APNs relay?",
        priority: "normal",
        deadline_at: "2026-10-07T16:00:00Z",
        relayed: false,
        raised_by: "arch",
      },
      {
        kind: "p0",
        id: "H-021",
        title: "Pairing crash on iOS 18.1",
        column_key: "doing",
        assignee: "ios",
      },
      {
        kind: "relayed",
        count: 2,
        decision_ids: ["dec-7", "dec-8"],
        by: [{ bot_id: "arch", count: 2 }],
        rulings: [
          {
            id: "dec-7",
            title: "Ship builds on Fridays?",
            answer: "Yes, before noon only.\nNever on a release week.",
            bot_id: "arch",
            at: "2026-10-04T08:30:00Z",
          },
          {
            id: "dec-8",
            title: "Keep the old app icon?",
            answer: "No",
            bot_id: "arch",
            at: "2026-10-04T11:15:00Z",
          },
        ],
      },
    ],
    wip_overrides: [
      {
        id: "H-030",
        title: "Hotfix the installer",
        column_key: "review",
        actor: "bot:dd",
        note: "WIP override: hotfix for 0.15.2",
        at: "2026-10-04T09:00:00Z",
      },
    ],
    needs_you_note: null,
    board: BOARD,
    releases: [
      waiting,
      release({
        id: "rel-0",
        name: "R-2026-W40",
        display_version: "0.15.2",
        status: "deployed",
        deployments: [
          deployment({ result: "ok" }),
          deployment({ machine: "win-pc", result: "ok" }),
        ],
      }),
    ],
    team: [
      { bot: DEV, items: [{ id: "H-002", title: "Safe destructive actions" }], open_tasks: 1 },
      { bot: WORKER, items: [{ id: "H-040", title: "Name the holders" }], open_tasks: 1 },
      { bot: IOS, items: [{ id: "H-021", title: "Pairing crash" }], open_tasks: 1 },
      { bot: TESTER, items: [], open_tasks: 1 },
      { bot: ARCH, items: [], open_tasks: 0 },
    ],
    meetings: MEETING_ROWS,
    action_items: ACTION_ITEMS,
    ...over,
  };
}

/** The board as a computer that mirrors it sees it: no history there. */
export const MIRRORED_BOARD: BoardSummary = {
  ...BOARD,
  done_this_week: null,
  rework_this_week: null,
};

/** The busy dashboard as a linked computer sees it: the home's rows (all but
 *  the P0, which the mirror holds) to act on there. */
export function offHome(over: Partial<Dashboard> = {}): Dashboard {
  const rows: NeedsYou[] = [];
  for (const row of dashboard().needs_you) {
    rows.push(row.kind === "p0" ? row : { ...row, elsewhere: "mac" });
  }
  return dashboard({ home: "mac", board: MIRRORED_BOARD, needs_you: rows, ...over });
}

/** A fresh project: nothing waits, no board, no release, no meetings. */
export function quietDashboard(): Dashboard {
  return dashboard({
    needs_you: [],
    wip_overrides: [],
    board: null,
    releases: [],
    team: [],
    meetings: [],
    action_items: [],
  });
}
