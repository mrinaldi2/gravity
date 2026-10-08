// Plain text with its card ids as links (UX-035 §7): for the surfaces that
// show text as it is, not Markdown (row details, toasts, notes).

import { Fragment } from "react";
import type { ReactElement } from "react";
import type { Segment } from "./cardIds";
import { splitCardIds } from "./cardIds";
import CardLink from "./CardLink";
import { useCardLinks } from "./CardLinks";

/** Each piece with where it starts in the text, its key. */
function placed(segments: readonly Segment[]): { readonly at: number; readonly s: Segment }[] {
  let at = 0;
  return segments.map((s) => {
    const start = at;
    at += s.kind === "text" ? s.text.length : s.id.length;
    return { at: start, s };
  });
}

export default function LinkedText({ text }: { readonly text: string }): ReactElement {
  const pattern = useCardLinks()?.pattern ?? null;
  return (
    <>
      {placed(splitCardIds(text, pattern)).map(({ at, s }) =>
        s.kind === "text" ? (
          <Fragment key={at}>{s.text}</Fragment>
        ) : (
          <CardLink key={at} id={s.id} />
        ),
      )}
    </>
  );
}
