// Owner-facing words for the board's enums (ux-glossary §5, H-018 §3.2).
// Every type carries a glyph and a word, so nothing depends on colour.

import { ItemType, Platform, Priority } from "../../protocol/gen/hermes/board/v1/board_pb";

interface TypeLabel {
  readonly glyph: string;
  readonly word: string;
}

const TYPE_LABEL: Readonly<Record<ItemType, TypeLabel>> = {
  [ItemType.UNSPECIFIED]: { glyph: "•", word: "Item" },
  [ItemType.EPIC]: { glyph: "◆", word: "Epic" },
  [ItemType.FEATURE]: { glyph: "▲", word: "Feature" },
  [ItemType.BUG]: { glyph: "✱", word: "Bug" },
  [ItemType.SPIKE]: { glyph: "◎", word: "Spike" },
  [ItemType.CHORE]: { glyph: "▪", word: "Chore" },
};

export function typeLabel(type: ItemType): TypeLabel {
  return TYPE_LABEL[type] ?? TYPE_LABEL[ItemType.UNSPECIFIED];
}

/** The types the Type filter offers; epics never appear as cards (H-017 §2.1). */
export const FILTER_TYPES: readonly ItemType[] = [
  ItemType.FEATURE,
  ItemType.BUG,
  ItemType.SPIKE,
  ItemType.CHORE,
];

const PLATFORM_WORD: Readonly<Record<Platform, string>> = {
  [Platform.UNSPECIFIED]: "other",
  [Platform.DESKTOP]: "desktop",
  [Platform.IOS]: "ios",
  [Platform.DAEMON]: "daemon",
  [Platform.INFRA]: "infra",
};

export function platformWord(platform: Platform): string {
  return PLATFORM_WORD[platform] ?? PLATFORM_WORD[Platform.UNSPECIFIED];
}

export const FILTER_PLATFORMS: readonly Platform[] = [
  Platform.DESKTOP,
  Platform.IOS,
  Platform.DAEMON,
  Platform.INFRA,
];

/** "P0" / "P1" for the priorities a card calls out; the rest stay quiet. */
export function urgentPriority(priority: Priority): string | null {
  if (priority === Priority.P0) {
    return "P0";
  }
  if (priority === Priority.P1) {
    return "P1";
  }
  return null;
}
