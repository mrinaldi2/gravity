// Moving items (H-018 §3.3). The board never invents rules: it asks the
// Hermes service (`item_move_check`) and shows what comes back.

import type { Unmet } from "../../protocol/gen/hermes/board/v1/board_pb";
import type { MoveCheck } from "../../protocol/gen/hermes/board/v1/requests_pb";

/** Guards the owner clears by typing something, rather than by changing the item. */
const REASON = "reason.required";
const WIP_FULL = "wip.full";

export type MovePlan =
  | { readonly kind: "go" }
  | {
      readonly kind: "input";
      readonly needsReason: boolean;
      readonly needsOverride: boolean;
      readonly unmet: readonly Unmet[];
    }
  | { readonly kind: "refused"; readonly unmet: readonly Unmet[] };

/** What a move to a column with these unmet guards takes. */
export function planMove(unmet: readonly Unmet[]): MovePlan {
  if (unmet.length === 0) {
    return { kind: "go" };
  }
  if (unmet.every((u) => u.code === REASON || u.code === WIP_FULL)) {
    return {
      kind: "input",
      needsReason: unmet.some((u) => u.code === REASON),
      needsOverride: unmet.some((u) => u.code === WIP_FULL),
      unmet,
    };
  }
  return { kind: "refused", unmet };
}

/** A column's guards, keyed by column key. */
export type ColumnChecks = ReadonlyMap<string, readonly Unmet[]>;

export function columnChecks(check: MoveCheck): ColumnChecks {
  return new Map(check.columns.map((column) => [column.columnKey, column.unmet]));
}

/** The reason chip on a column header: the first unmet guard, plus "+N more". */
export function reasonChip(unmet: readonly Unmet[]): string {
  const [first] = unmet;
  if (first === undefined) {
    return "";
  }
  return unmet.length > 1 ? `${first.text} +${unmet.length - 1} more` : first.text;
}

/** How long a move check stays fresh during drags (H-018 §3.3). */
export const CHECK_TTL_MS = 10_000;

interface CachedCheck {
  readonly version: bigint;
  readonly at: number;
  readonly checks: ColumnChecks;
}

/** Move checks per item, dropped when the item's version moves on or they age out. */
export class MoveCheckCache {
  private readonly entries = new Map<string, CachedCheck>();

  get(itemId: string, version: bigint, now: number): ColumnChecks | undefined {
    const entry = this.entries.get(itemId);
    if (entry === undefined || entry.version !== version || now - entry.at > CHECK_TTL_MS) {
      return undefined;
    }
    return entry.checks;
  }

  set(itemId: string, version: bigint, now: number, checks: ColumnChecks): void {
    this.entries.set(itemId, { version, at: now, checks });
  }

  clear(): void {
    this.entries.clear();
  }
}
