// Needs-you rows from another linked computer (H-178): what the projects
// home counts there, listed here too so both say the same. Each is acted on
// that computer: the row names it in place of a button.

import type { AttentionRowJson, TypedRowJson } from "../../protocol/dashboard";
import type { AttentionActions } from "./attentionRows";
import { attentionRow } from "./attentionRows";
import type { RowViewProps } from "./NeedsYou";

/** The kinds the dashboard's own rows have, as their typed names. */
const PLAIN: Readonly<
  Record<
    Exclude<TypedRowJson["kind"], AttentionRowJson["kind"]>,
    { readonly glyph: string; readonly meta: string; readonly verb: string; readonly bad?: true }
  >
> = {
  release_awaiting: { glyph: "▣", meta: "A release waits for your review", verb: "Review" },
  decision: { glyph: "?", meta: "A decision waits for you", verb: "Answer" },
  relayed_rulings: { glyph: "✓", meta: "Rulings a bot recorded for you", verb: "Confirm" },
  p0_item: { glyph: "!", meta: "P0", verb: "Open", bad: true },
  serving_off: { glyph: "⚠", meta: "Builds aren't being published", verb: "Fix" },
};

function isAttention(row: TypedRowJson): row is AttentionRowJson {
  return !(row.kind in PLAIN);
}

/** The row's view; the button gives way to "<verb> on <computer>". */
export function elsewhereRow(row: TypedRowJson, a: AttentionActions): RowViewProps | undefined {
  if (isAttention(row)) {
    return attentionRow(row, a);
  }
  const plain = PLAIN[row.kind as keyof typeof PLAIN];
  return {
    glyph: plain.glyph,
    tone: plain.bad === true ? "bad" : "you",
    title: row.title,
    meta: plain.meta,
    verb: plain.verb,
  };
}

/** Where the row sorts among the dashboard's own kinds. */
export function elsewhereKind(row: TypedRowJson): string {
  switch (row.kind) {
    case "release_awaiting":
      return "release";
    case "relayed_rulings":
      return "relayed";
    case "p0_item":
      return "p0";
    default:
      return row.kind;
  }
}
