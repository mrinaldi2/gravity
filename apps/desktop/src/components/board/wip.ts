// WIP limits in column headers (H-018 §3.4). The state is always carried by
// a word ("Full", "Over"), never by colour alone.

import type { BoardColumn, ItemCard } from "../../protocol/gen/hermes/board/v1/board_pb";
import { WipScope } from "../../protocol/gen/hermes/board/v1/board_pb";

type WipState = "none" | "under" | "full" | "over";

interface AssigneeLoad {
  readonly botId: string;
  readonly count: number;
}

export interface WipSummary {
  /** Cards in the column. */
  readonly total: number;
  /** Cards in the column that pass the filters. */
  readonly shown: number;
  readonly limit: number | undefined;
  readonly perAssignee: boolean;
  readonly state: WipState;
  /** Per-assignee counts, busiest first; only for a per-assignee limit. */
  readonly loads: readonly AssigneeLoad[];
}

function stateOf(count: number, limit: number): WipState {
  if (count > limit) {
    return "over";
  }
  return count === limit ? "full" : "under";
}

const RANK: Readonly<Record<WipState, number>> = { none: 0, under: 1, full: 2, over: 3 };

export function wipSummary(
  column: BoardColumn,
  cards: readonly ItemCard[],
  shown: number,
): WipSummary {
  const limit = column.wipLimit;
  const perAssignee = limit !== undefined && column.wipScope === WipScope.PER_ASSIGNEE;
  const base = { total: cards.length, shown, limit, perAssignee };
  if (limit === undefined) {
    return { ...base, state: "none", loads: [] };
  }
  if (!perAssignee) {
    return { ...base, state: stateOf(cards.length, limit), loads: [] };
  }
  const counts = new Map<string, number>();
  for (const card of cards) {
    if (card.assignee !== undefined) {
      counts.set(card.assignee, (counts.get(card.assignee) ?? 0) + 1);
    }
  }
  const loads = [...counts].map(([botId, count]) => ({ botId, count }));
  // oxlint-disable-next-line unicorn/no-array-sort
  loads.sort((a, b) => b.count - a.count || a.botId.localeCompare(b.botId));
  const state = loads.reduce<WipState>((worst, load) => {
    const next = stateOf(load.count, limit);
    return RANK[next] > RANK[worst] ? next : worst;
  }, "under");
  return { ...base, state, loads };
}

/** "5", or "2 of 5" while filters hide some cards. */
export function countText(summary: WipSummary): string {
  return summary.shown === summary.total
    ? String(summary.total)
    : `${summary.shown} of ${summary.total}`;
}

/** "Review, 4 items, limit 3, over limit by 1": the header's tooltip and accessible name. */
export function wipDescription(name: string, summary: WipSummary): string {
  const items = `${summary.total} ${summary.total === 1 ? "item" : "items"}`;
  const parts = [name, items];
  if (summary.shown !== summary.total) {
    parts.push(`${summary.shown} shown`);
  }
  if (summary.limit !== undefined) {
    parts.push(summary.perAssignee ? `limit ${summary.limit} per bot` : `limit ${summary.limit}`);
    if (summary.state === "full") {
      parts.push("full");
    } else if (summary.state === "over") {
      parts.push(
        summary.perAssignee
          ? "a bot is over its limit"
          : `over limit by ${summary.total - summary.limit}`,
      );
    }
  }
  return parts.join(", ");
}
