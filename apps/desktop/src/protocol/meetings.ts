// Meetings and action items over the JSON WebSocket (H-102, H-020 §4): the
// dashboard's widgets 5 and 6, and the owner ticking, dropping or promoting
// an action. Ids of bots are bot ids; the owner is "owner".

/** The owner, where an attendee or action owner would name a bot. */
export const OWNER = "owner";

type MeetingStatus = "scheduled" | "collecting" | "held" | "skipped";
export type ActionStatus = "open" | "done" | "dropped";

export interface MeetingSeries {
  readonly id: string;
  /** standup, refinement, demo, retro or adhoc; text from newer daemons. */
  readonly type: string;
  readonly name: string;
  readonly cron: string;
  readonly tz: string;
  readonly facilitator: string;
  readonly attendees: readonly string[];
  readonly enabled: boolean;
}

/** A meeting as lists show it: no contributions or inputs. */
export interface MeetingSummary {
  readonly id: string;
  readonly series_id: string | null;
  readonly type: string;
  readonly name: string;
  readonly status: MeetingStatus;
  readonly skip_reason: string | null;
  readonly started_at: string | null;
  readonly closed_at: string | null;
  readonly facilitator: string;
  /** The facilitator's, at most ten lines. */
  readonly summary: string;
  /** By section, e.g. `{"blockers": "…"}`. */
  readonly outputs: Readonly<Record<string, string>>;
  readonly contributed: number;
  readonly attendee_count: number;
}

/** One series, or an ad-hoc meeting still collecting (`series` null). */
export interface MeetingRow {
  readonly series: MeetingSeries | null;
  /** When its routine next starts it; null while disabled. */
  readonly next_at: string | null;
  readonly collecting: MeetingSummary | null;
  readonly last_held: MeetingSummary | null;
}

interface ActionItem {
  readonly id: string;
  readonly meeting_id: string;
  readonly series_id: string | null;
  readonly text: string;
  /** A bot id, or "owner". */
  readonly owner: string;
  readonly due_at: string | null;
  readonly status: ActionStatus;
  /** The chore it was promoted to. */
  readonly item_id: string | null;
}

/** An open action as the dashboard lists it. */
export interface DashboardAction extends ActionItem {
  readonly meeting_name: string | null;
  readonly overdue: boolean;
}

export type MeetingRequestBody =
  | {
      readonly type: "action_update";
      readonly project_id: string;
      readonly action_id: string;
      readonly status?: ActionStatus;
      readonly text?: string;
      /** RFC 3339 or YYYY-MM-DD; empty clears it. */
      readonly due_at?: string;
    }
  /** A chore on the board, linked to the action's meeting. */
  | {
      readonly type: "action_promote";
      readonly project_id: string;
      readonly action_id: string;
      readonly title?: string;
    };

export type MeetingReply = { readonly type: "meeting_action"; readonly action: ActionItem };
