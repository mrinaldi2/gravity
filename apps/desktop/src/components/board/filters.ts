// The board's filter bar (H-018 §3.5): bot, platform and type. Filtered-out
// cards are hidden, but column counts still read "2 of 5".

import type { ItemCard } from "../../protocol/gen/hermes/board/v1/board_pb";
import { ItemType, Platform } from "../../protocol/gen/hermes/board/v1/board_pb";

export interface BoardFilters {
  /** Assignee bot ids. */
  readonly bots: readonly string[];
  readonly platforms: readonly Platform[];
  readonly types: readonly ItemType[];
}

export const NO_FILTERS: BoardFilters = { bots: [], platforms: [], types: [] };

export function isFiltering(filters: BoardFilters): boolean {
  return filters.bots.length + filters.platforms.length + filters.types.length > 0;
}

/** Every set filter must match; platforms match on any overlap. */
export function matchesFilters(card: ItemCard, filters: BoardFilters): boolean {
  if (
    filters.bots.length > 0 &&
    (card.assignee === undefined || !filters.bots.includes(card.assignee))
  ) {
    return false;
  }
  if (
    filters.platforms.length > 0 &&
    !card.platforms.some((platform) => filters.platforms.includes(platform))
  ) {
    return false;
  }
  return filters.types.length === 0 || filters.types.includes(card.type);
}

/** Adds `value` to `list`, or removes it when present. */
export function toggled<T>(list: readonly T[], value: T): readonly T[] {
  return list.includes(value) ? list.filter((item) => item !== value) : [...list, value];
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function numbersIn<T extends number>(value: unknown, known: (n: number) => n is T): readonly T[] {
  return Array.isArray(value)
    ? value.filter((n): n is T => typeof n === "number" && known(n))
    : [];
}

const isPlatform = (n: number): n is Platform => n in Platform && n !== Platform.UNSPECIFIED;
const isItemType = (n: number): n is ItemType => n in ItemType && n !== ItemType.UNSPECIFIED;

/** Narrows stored JSON back to filters, dropping anything unknown. */
export function parseFilters(value: unknown): BoardFilters {
  if (!isRecord(value)) {
    return NO_FILTERS;
  }
  const bots = Array.isArray(value["bots"])
    ? value["bots"].filter((id): id is string => typeof id === "string")
    : [];
  return {
    bots,
    platforms: numbersIn(value["platforms"], isPlatform),
    types: numbersIn(value["types"], isItemType),
  };
}
