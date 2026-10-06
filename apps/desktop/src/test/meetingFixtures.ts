// Meetings and action items for the dashboard's tests and stories (H-102).

import type { ServerReply } from "../protocol/messages";
import type {
  DashboardAction,
  MeetingDetail,
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

/** The Meetings tab's two meetings: a held stand-up, a retro still collecting. */
const TAB_MEETINGS: readonly MeetingSummary[] = [
  meeting({
    id: "m1",
    name: "Stand-up",
    facilitator: "b1",
    summary: "Verify is full; testers clear H-008 and H-010 first.",
    outputs: { blockers: "H-013 push budget: who can unblock: you" },
  }),
  meeting({
    id: "m2",
    series_id: "s-retro",
    type: "retro",
    name: "Retro W41",
    status: "collecting",
    closed_at: null,
    summary: "",
    outputs: {},
    contributed: 4,
    attendee_count: 6,
  }),
];

/** `meeting_list` for the Meetings tab. */
export function meetingListReply(): ServerReply {
  return {
    type: "meetings",
    req_id: "1",
    series: [{ ...series({ name: "Stand-up" }), next_at: "2026-10-07T07:00:00Z" }],
    meetings: TAB_MEETINGS,
  };
}

/** `meeting_get` for one of the Meetings tab's meetings. */
export function meetingDetail(id: string): MeetingDetail {
  const summary = TAB_MEETINGS.find((m) => m.id === id) ?? meeting();
  return {
    ...summary,
    scheduled_at: null,
    contributions: [],
    action_items:
      id === "m1"
        ? [
            {
              id: "a1",
              meeting_id: "m1",
              series_id: "s-standup",
              text: "Split H-014",
              owner: "b1",
              due_at: null,
              status: "open",
              item_id: null,
            },
          ]
        : [],
    carried_over: [],
  };
}
