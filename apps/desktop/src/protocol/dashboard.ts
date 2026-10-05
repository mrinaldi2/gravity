// The project dashboard's data over the JSON WebSocket (H-076, H-018 §2.1):
// what its widgets show, in one `dashboard_get`. Meetings and action items
// come from H-102.

import type { Bot } from "./entities";
import type { DashboardAction, MeetingRow } from "./meetings";
import type { Release } from "./releases";

/** A move made over a WIP limit: the lead's call, listed beside Needs you. */
export interface WipOverride {
  readonly id: string;
  readonly title: string;
  readonly column_key: string;
  readonly actor: string;
  readonly note: string;
  readonly at: string;
}

/** One ruling a bot recorded for the owner, as the confirm dialog lists it. */
export interface RelayedRuling {
  readonly id: string;
  readonly title: string;
  /** The recorded answer, verbatim. */
  readonly answer: string;
  /** The bot that recorded it. */
  readonly bot_id: string;
  readonly at: string | null;
}

type Row =
  | { readonly kind: "release"; readonly release: Release }
  | {
      readonly kind: "decision";
      readonly id: string;
      readonly title: string;
      readonly priority: string;
      readonly deadline_at: string | null;
      /** A bot's relay of a ruling; daemons before 0.16.2 list each one. */
      readonly relayed: boolean;
      /** The bot that raised it; absent from daemons before 0.16.1. */
      readonly raised_by?: string;
    }
  | {
      /** Every ruling a bot recorded for the owner, confirmed together (H-112). */
      readonly kind: "relayed";
      readonly count: number;
      readonly decision_ids: readonly string[];
      readonly by: readonly { readonly bot_id: string; readonly count: number }[];
      /** Each one, for the dialog that confirms them (ARCH-R42 M1). */
      readonly rulings: readonly RelayedRuling[];
    }
  | {
      readonly kind: "p0";
      readonly id: string;
      readonly title: string;
      readonly column_key: string;
      readonly assignee: string | null;
    }
  /** In Needs you on daemons before 0.16.2; shown beside it here. */
  | ({ readonly kind: "wip_override" } & WipOverride);

/** A row, and off the board's home the computer to act on it (H-112). */
export type NeedsYou = Row & { readonly elsewhere?: string };

interface ColumnSummary {
  readonly key: string;
  readonly name: string;
  readonly category: string;
  readonly count: number;
  readonly wip_limit: number | null;
  readonly wip_scope: "column" | "per_assignee";
}

export interface BoardSummary {
  readonly columns: readonly ColumnSummary[];
  readonly blocked: number;
  readonly stale: number;
  /** Since `since`, imported items aside; null off the board's home. */
  readonly done_this_week: number | null;
  readonly rework_this_week: number | null;
}

export interface TeamRow {
  readonly bot: Bot;
  /** Its items in Doing, in board order. */
  readonly items: readonly { readonly id: string; readonly title: string }[];
  readonly open_tasks: number;
}

export interface Dashboard {
  readonly project_id: string;
  readonly as_of: string;
  /** Where "this week" starts: seven days before `as_of`. */
  readonly since: string;
  /** The computer holding the board, when it is mirrored here. */
  readonly home: string | null;
  readonly needs_you: readonly NeedsYou[];
  /** Moves over a WIP limit this week; absent before 0.16.2. */
  readonly wip_overrides?: readonly WipOverride[];
  /** Off-home, why the home's rows are missing (it can't be reached). */
  readonly needs_you_note?: string | null;
  /** Null when the project has no board. */
  readonly board: BoardSummary | null;
  /** The current release and the last two, newest first. */
  readonly releases: readonly Release[];
  readonly team: readonly TeamRow[];
  /** Each series, then ad-hoc meetings still collecting; empty off-home. */
  readonly meetings: readonly MeetingRow[];
  /** Open action items, soonest due first. */
  readonly action_items: readonly DashboardAction[];
}

export type DashboardRequestBody =
  | { readonly type: "dashboard_get"; readonly project_id: string }
  /** Confirms those of the listed relayed rulings that still are; needs approve. */
  | {
      readonly type: "confirm_relayed";
      readonly project_id: string;
      readonly decision_ids: readonly string[];
    };

export type DashboardReply =
  | { readonly type: "dashboard"; readonly dashboard: Dashboard }
  | {
      readonly type: "relayed_confirmed";
      readonly confirmed: readonly string[];
      readonly failed: readonly { readonly id: string; readonly message: string }[];
      /** Listed but no longer relayed: the list changed, so reread it. */
      readonly changed: readonly string[];
    };
