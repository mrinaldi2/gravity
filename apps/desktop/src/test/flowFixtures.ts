// The Flow widget's numbers for tests and stories (B11).

import type { FlowDay, FlowMetrics } from "../protocol/metrics";

const DAY = 86_400;

function days(n: number): FlowDay[] {
  const wip = [2, 3, 3, 4, 3, 2, 3, 4, 4, 3, 2, 3, 3, 4, 5, 4, 3, 3, 2, 3, 4, 4, 3, 3, 2, 3, 3, 3];
  return Array.from({ length: n }, (_, i) => ({
    at: new Date(Date.UTC(2026, 8, 8 + (28 - n) + i, 10, 42)).toISOString(),
    wip: wip[(28 - n + i) % wip.length] ?? 0,
    columns: { doing: 2, review: 1 },
  }));
}

/** A working week on the board. */
export function flowMetrics(over: Partial<FlowMetrics> = {}): FlowMetrics {
  return {
    since: "2026-09-28T10:42:00Z",
    days: 7,
    throughput: 7,
    weekly: [3, 5, 4, 6, 2, 5, 6, 7],
    cycle: { p50: 1.6 * DAY, p85: 3.2 * DAY, count: 7 },
    by_column: [
      { key: "doing", name: "Doing", p50: 1.2 * DAY, p85: 2.4 * DAY, count: 9 },
      { key: "review", name: "Review", p50: 0.5 * DAY, p85: 1.1 * DAY, count: 8 },
      { key: "verify", name: "Verify", p50: 4 * 3600, p85: 0.6 * DAY, count: 7 },
      { key: "approval", name: "Awaiting owner", p50: 1.8 * DAY, p85: 3.0 * DAY, count: 4 },
    ],
    daily: days(7),
    reworked: 1,
    past_doing: 8,
    rework_rate: 0.125,
    aging: [
      { id: "H-014", title: "Pairing over Tailscale", column_key: "review", age: 3.1 * DAY },
      { id: "H-009", title: "Board drag and drop", column_key: "doing", age: 2.2 * DAY },
    ],
    expired_tasks: 2,
    ...over,
  };
}

/** Four weeks of the same. */
export function flowMetrics4w(): FlowMetrics {
  return flowMetrics({ days: 28, throughput: 20, daily: days(28) });
}

/** A board where nothing has reached Done yet. */
export function emptyFlow(): FlowMetrics {
  return flowMetrics({
    throughput: 0,
    weekly: [0, 0, 0, 0, 0, 0, 0, 0],
    cycle: { p50: 0, p85: 0, count: 0 },
    reworked: 0,
    past_doing: 0,
    rework_rate: 0,
    aging: [],
    expired_tasks: 0,
  });
}
