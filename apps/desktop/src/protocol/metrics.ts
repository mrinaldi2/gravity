// The dashboard's Flow numbers (B11, `metrics_get`): computed by the daemon
// from the board's history, imported items aside. Times are in seconds.

export type MetricsRange = "week" | "4w";

interface Spread {
  readonly p50: number;
  readonly p85: number;
  readonly count: number;
}

export interface ColumnTime extends Spread {
  readonly key: string;
  readonly name: string;
}

export interface FlowDay {
  /** The end of the day the counts are taken at. */
  readonly at: string;
  /** Items in progress, Doing through Deploying. */
  readonly wip: number;
  readonly columns: Readonly<Record<string, number>>;
}

interface AgingItem {
  readonly id: string;
  readonly title: string;
  readonly column_key: string;
  readonly age: number;
}

export interface FlowMetrics {
  readonly since: string;
  readonly days: number;
  readonly throughput: number;
  /** Done per week for 8 weeks, oldest first. */
  readonly weekly: readonly number[];
  readonly cycle: Spread;
  readonly by_column: readonly ColumnTime[];
  readonly daily: readonly FlowDay[];
  readonly reworked: number;
  readonly past_doing: number;
  readonly rework_rate: number;
  readonly aging: readonly AgingItem[];
  readonly expired_tasks: number;
}

export type MetricsRequestBody = {
  readonly type: "metrics_get";
  readonly project_id: string;
  readonly range: MetricsRange;
};

export type MetricsReply = {
  readonly type: "metrics";
  /** Null without a board, or off-home when the home is away. */
  readonly metrics: FlowMetrics | null;
  /** Off-home: where the numbers are kept, when they can't be read. */
  readonly note: string | null;
};
