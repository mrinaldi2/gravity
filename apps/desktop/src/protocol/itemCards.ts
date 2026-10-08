// Card ids as links (H-203, UX-035 §9): `item_cards_get` answers each id with
// its card and where it is kept, or why it can't. Behind the `item_cards`
// capability; an older service answers neither.

/** One id's answer. `card` is the ItemCard in proto3 JSON. */
export interface ItemCardEntry {
  readonly id: string;
  readonly project_id?: string | null;
  readonly project_name?: string | null;
  /** The computer that keeps the card's board. */
  readonly computer?: string | null;
  readonly card?: ItemCardJson | null;
  /** The column's name as the board shows it. */
  readonly column_name?: string | null;
  /** The release it is in, by its version. */
  readonly release?: string | null;
  /** No board has it: deleted, or mistyped. */
  readonly missing?: boolean | null;
  /** Kept on a computer that is offline; `card` is the last copy, if any. */
  readonly unreachable?: {
    readonly computer: string;
    readonly last_seen?: string | null;
  } | null;
}

/** The fields of an ItemCard the preview reads (proto3 JSON). */
interface ItemCardJson {
  readonly id: string;
  readonly type?: string;
  readonly title?: string;
  readonly priority?: string;
  readonly columnKey?: string;
  readonly assignee?: string;
  readonly blocked?: boolean;
}

export type ItemCardsRequestBody = {
  readonly type: "item_cards_get";
  readonly ids: readonly string[];
};

export type ItemCardsReply = {
  readonly type: "item_cards";
  readonly cards: readonly ItemCardEntry[];
};
