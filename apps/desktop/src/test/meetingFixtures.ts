// Meetings and action items for the dashboard's tests and stories (H-102).

import type {
  DashboardAction,
  MeetingRow,
  MeetingSeries,
  MeetingSummary,
} from "../protocol/meetings";

function series(over: Partial<MeetingSeries>): MeetingSeries {
  return {
    id: "s-standup",
    type: "standup",
    name: "Daily standup",
    cron: "0 0 9 * * Mon-Fri",
    tz: "Europe/Rome",
    facilitator: "arch",
    attendees: ["dd", "ios", "owner"],
    enabled: true,
    ...over,
  };
}

function meeting(over: Partial<MeetingSummary> = {}): MeetingSummary {
  return {
    id: "MTG-2026-10-05-standup",
    series_id: "s-standup",
    type: "standup",
    name: "Daily standup",
    status: "held",
    skip_reason: null,
    started_at: "2026-10-05T07:00:00Z",
    closed_at: "2026-10-05T08:10:00Z",
    facilitator: "arch",
    summary: "2 blockers: H-021 needs a device, H-014 waits on review.\nEveryone else on track.",
    outputs: { blockers: "H-021, H-014" },
    contributed: 3,
    attendee_count: 3,
    ...over,
  };
}

export const MEETING_ROWS: readonly MeetingRow[] = [
  {
    series: series({}),
    next_at: "2026-10-06T07:00:00Z",
    collecting: null,
    last_held: meeting(),
  },
  {
    series: series({ id: "s-retro", type: "retro", name: "Retro", cron: "0 0 16 * * Fri" }),
    next_at: "2026-10-09T14:00:00Z",
    collecting: meeting({
      id: "MTG-2026-10-05-retro",
      series_id: "s-retro",
      type: "retro",
      name: "Retro",
      status: "collecting",
      closed_at: null,
      summary: "",
      outputs: {},
      contributed: 4,
      attendee_count: 6,
    }),
    last_held: null,
  },
];

function action(over: Partial<DashboardAction>): DashboardAction {
  return {
    id: "a1",
    meeting_id: "MTG-2026-10-05-standup",
    series_id: "s-standup",
    text: "Split H-014",
    owner: "arch",
    due_at: "2026-10-03T21:59:59Z",
    status: "open",
    item_id: null,
    meeting_name: "Daily standup",
    overdue: true,
    ...over,
  };
}

export const ACTION_ITEMS: readonly DashboardAction[] = [
  action({}),
  action({
    id: "a2",
    text: "Decide the release cadence",
    owner: "owner",
    due_at: "2026-10-08T21:59:59Z",
    overdue: false,
  }),
  action({
    id: "a3",
    text: "Add a WIP alert",
    owner: "dd",
    due_at: null,
    item_id: "H-120",
    overdue: false,
  }),
];
