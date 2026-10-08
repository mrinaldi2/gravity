// What a card link's preview says (UX-035 §3, §6, §8): the card in four
// lines, or where it lives and why it can't be shown. Never "not found" or
// "error" on their own; "computer" and "Hermes service", not "home".

import { fromJson } from "@bufbuild/protobuf";
import type { JsonValue } from "@bufbuild/protobuf";
import { ItemCardSchema, ItemType, Priority } from "../../protocol/gen/hermes/board/v1/board_pb";
import type { ItemCardEntry } from "../../protocol/itemCards";

export type Preview =
  | { readonly kind: "loading"; readonly text: string }
  | {
      readonly kind: "card";
      /** `H-293 · Bug · P0` */
      readonly head: string;
      readonly title: string;
      /** `Doing · Desktop Dev` */
      readonly where: string;
      /** `⛔ Blocked · in 0.17.3`, when either applies. */
      readonly extra: string | null;
      /** The project, when it isn't the one on screen. */
      readonly project: string | null;
    }
  /** Can't be shown: why, and the title last seen if there was one. */
  | { readonly kind: "note"; readonly text: string; readonly lastSeen: string | null };

const TYPES: Readonly<Partial<Record<ItemType, string>>> = {
  [ItemType.EPIC]: "Epic",
  [ItemType.FEATURE]: "Feature",
  [ItemType.BUG]: "Bug",
  [ItemType.SPIKE]: "Spike",
  [ItemType.CHORE]: "Chore",
};

/** Only the urgent priorities are worth the space. */
const PRIORITIES: Readonly<Partial<Record<Priority, string>>> = {
  [Priority.P0]: "P0",
  [Priority.P1]: "P1",
};

function clock(iso: string | null | undefined): string | null {
  if (!iso) {
    return null;
  }
  const at = new Date(iso);
  return Number.isNaN(at.getTime())
    ? null
    : at.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
}

export interface PreviewContext {
  readonly botName: (id: string) => string | undefined;
  /** The project on screen: its cards don't name it. */
  readonly currentProjectId: string | null;
  /** The title this id had in an earlier answer. */
  readonly lastTitle: string | undefined;
}

/** The preview for `id` from its answer, or `undefined` while it loads. */
export function preview(
  id: string,
  entry: ItemCardEntry | undefined,
  ctx: PreviewContext,
): Preview {
  if (entry === undefined) {
    return { kind: "loading", text: `Loading ${id}…` };
  }
  return cantShow(id, entry, ctx) ?? cardPreview(id, entry, ctx);
}

/** Why `id` can't be shown from here, or `null` when it can. */
function cantShow(id: string, entry: ItemCardEntry, ctx: PreviewContext): Preview | null {
  if (entry.unreachable) {
    const project = entry.project_name ?? "its project";
    const seen = clock(entry.unreachable.last_seen);
    const computer = entry.unreachable.computer;
    const when = seen ? ` · last seen ${seen}` : "";
    return {
      kind: "note",
      text: `${id} is on ${project}'s board, kept on ${computer}. ${computer} is offline${when}.`,
      lastSeen: entry.card?.title ?? ctx.lastTitle ?? null,
    };
  }
  if (entry.missing || !entry.card) {
    const where = entry.project_name ? `${entry.project_name}'s board` : "any board here";
    return {
      kind: "note",
      text: `${id} isn't on ${where}. It may have been deleted or mistyped.`,
      lastSeen: null,
    };
  }
  return null;
}

function cardPreview(id: string, entry: ItemCardEntry, ctx: PreviewContext): Preview {
  const card = fromJson(ItemCardSchema, entry.card as unknown as JsonValue, {
    ignoreUnknownFields: true,
  });
  const head = [id, TYPES[card.type], PRIORITIES[card.priority]].filter(Boolean).join(" · ");
  const assignee = card.assignee ? (ctx.botName(card.assignee) ?? "a bot") : "Unassigned";
  const extra = [card.blocked ? "⛔ Blocked" : null, entry.release ? `in ${entry.release}` : null]
    .filter(Boolean)
    .join(" · ");
  return {
    kind: "card",
    head,
    title: card.title,
    where: `${entry.column_name ?? card.columnKey} · ${assignee}`,
    extra: extra || null,
    project: entry.project_id === ctx.currentProjectId ? null : (entry.project_name ?? null),
  };
}

/** The note an older service gets in place of a preview. */
export function oldServiceNote(computer: string): Preview {
  return {
    kind: "note",
    text: `Update the Hermes service on ${computer} to see cards from here.`,
    lastSeen: null,
  };
}

/** The note when the service didn't answer in time. */
export function noAnswerNote(id: string): Preview {
  return {
    kind: "note",
    text: `${id} can't be looked up right now: the Hermes service didn't answer.`,
    lastSeen: null,
  };
}

/** The link's accessible name: the id first, then the title once known. */
export function linkName(id: string, p: Preview): string {
  return p.kind === "card" && p.title ? `${id}: ${p.title}` : `${id}, card`;
}
