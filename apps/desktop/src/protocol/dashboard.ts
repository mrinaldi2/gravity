// The project dashboard's data over the JSON WebSocket (H-076, H-018 §2.1):
// what widgets 1–4 show, in one `dashboard_get`. Meetings and action items
// are empty until meetings land (H-102).

import type { Bot } from "./entities";
import type { Release } from "./releases";

export type NeedsYou =
  | { readonly kind: "release"; readonly release: Release }
  | {
      readonly kind: "decision";
      readonly id: string;
      readonly title: string;
      readonly priority: string;
      readonly deadline_at: string | null;
      /** A bot's relay of a ruling, waiting for the owner to confirm it. */
      readonly relayed: boolean;
      /** The bot that raised it; absent from daemons before 0.16.1. */
      readonly raised_by?: string;
    }
  | {
      readonly kind: "p0";
      readonly id: string;
      readonly title: string;
      readonly column_key: string;
      readonly assignee: string | null;
    }
  | {
      readonly kind: "wip_override";
      readonly id: string;
      readonly title: string;
      readonly column_key: string;
      readonly actor: string;
      readonly note: string;
      readonly at: string;
    };

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
  /** Null when the project has no board. */
  readonly board: BoardSummary | null;
  /** The current release and the last two, newest first. */
  readonly releases: readonly Release[];
  readonly team: readonly TeamRow[];
  readonly meetings: readonly never[];
  readonly action_items: readonly never[];
}

export type DashboardRequestBody = { readonly type: "dashboard_get"; readonly project_id: string };

export type DashboardReply = { readonly type: "dashboard"; readonly dashboard: Dashboard };
