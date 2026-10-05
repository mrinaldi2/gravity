// The item drawer's words (H-018 §3.6): the state stepper, the next step
// from the guard check, and one line per history event. Every state is a
// word, never colour alone.

import { timestampDate } from "@bufbuild/protobuf/wkt";
import type { Timestamp } from "@bufbuild/protobuf/wkt";
import type { BoardColumn, ItemEvent, Unmet } from "../../../protocol/gen/hermes/board/v1/board_pb";
import { ColumnCategory, ItemEventKind } from "../../../protocol/gen/hermes/board/v1/board_pb";
import type { MoveCheck } from "../../../protocol/gen/hermes/board/v1/requests_pb";

/** The flow's steps, in order; Cancelled is off the path. */
export const STEPS: readonly { readonly category: ColumnCategory; readonly word: string }[] = [
  { category: ColumnCategory.INBOX, word: "Inbox" },
  { category: ColumnCategory.READY, word: "Ready" },
  { category: ColumnCategory.DOING, word: "Doing" },
  { category: ColumnCategory.REVIEW, word: "Review" },
  { category: ColumnCategory.VERIFY, word: "Verify" },
  { category: ColumnCategory.APPROVAL, word: "Awaiting owner" },
  { category: ColumnCategory.DEPLOYING, word: "Deploying" },
  { category: ColumnCategory.DONE, word: "Done" },
];

export function when(at: Timestamp | undefined): string {
  if (at === undefined) {
    return "";
  }
  return timestampDate(at).toLocaleString([], {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

/** "31h", "3d": how long since `at`, for "Doing for 31h". */
export function age(at: Timestamp | undefined, now: number): string {
  if (at === undefined) {
    return "";
  }
  const hours = Math.max(0, Math.floor((now - timestampDate(at).getTime()) / 3_600_000));
  return hours < 48 ? `${hours}h` : `${Math.floor(hours / 24)}d`;
}

export interface NextStep {
  /** The column the next forward move goes to, when there is one. */
  readonly to: BoardColumn | undefined;
  /** What stands in the way, for this connection. */
  readonly unmet: readonly Unmet[];
}

/** The next forward move and its unmet guards (`item_move_check`). */
export function nextStep(
  columns: readonly BoardColumn[],
  current: BoardColumn | undefined,
  check: MoveCheck | null,
): NextStep {
  const at = STEPS.findIndex((s) => s.category === current?.category);
  const following = at < 0 ? undefined : STEPS[at + 1];
  const to = following ? columns.find((c) => c.category === following.category) : undefined;
  const unmet = to ? (check?.columns.find((c) => c.columnKey === to.key)?.unmet ?? []) : [];
  return { to, unmet };
}

interface Words {
  readonly actor: string;
  readonly who: (actor: string) => string;
  readonly columnName: (key: string) => string;
}

/** " (note)", ": note", or nothing. */
function noted(note: string | undefined, lead: string): string {
  return note ? `${lead}${note}` : "";
}

/** Each kind of history event in words; anything else falls back below. */
const EVENT_WORDS: Partial<Record<ItemEventKind, (e: ItemEvent, w: Words) => string>> = {
  [ItemEventKind.CREATED]: (e, w) => `${w.actor} created it${noted(e.note, " · ")}`,
  [ItemEventKind.MOVED]: (e, w) =>
    `${w.actor} moved ${w.columnName(e.from ?? "")} → ${w.columnName(e.to ?? "")}${noted(e.note, ": ")}`,
  [ItemEventKind.EDITED]: (e, w) => `${w.actor} changed ${e.field ?? "it"}${noted(e.to, " to ")}`,
  [ItemEventKind.LINKED]: (e, w) => `${w.actor} linked ${e.field ?? "a"}${noted(e.to, " ")}`,
  [ItemEventKind.ASSIGNED]: (e, w) =>
    e.to ? `${w.actor} assigned ${w.who(`bot:${e.to}`)}` : `${w.actor} unassigned it`,
  [ItemEventKind.BLOCKED]: (e, w) =>
    e.note ? `${w.actor} blocked it: ${e.note}` : `${w.actor} unblocked it`,
  [ItemEventKind.RANKED]: (_e, w) => `${w.actor} reordered it`,
  [ItemEventKind.COMMENTED]: (_e, w) => `${w.actor} commented`,
};

/** "Team Lead moved Ready → Doing": one history event in words. */
export function eventLine(
  event: ItemEvent,
  who: (actor: string) => string,
  columnName: (key: string) => string,
): string {
  const words = { actor: who(event.actor), who, columnName };
  const line = EVENT_WORDS[event.kind];
  return line ? line(event, words) : `${words.actor}: ${event.note ?? "changed it"}`;
}
