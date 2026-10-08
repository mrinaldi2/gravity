// Which words are card ids (UX-035 §1): `PREFIX-n`, where PREFIX is an item
// prefix this computer knows and n is 1–5 digits, standing as a word of its
// own. So `UTF-8` and `SHA-256` stay text unless a project uses that prefix,
// and `H-189-project-clicks` (a branch) isn't linked.

/** A piece of text: plain, or a card id. */
export type Segment =
  | { readonly kind: "text"; readonly text: string }
  | { readonly kind: "id"; readonly id: string };

function escape(prefix: string): string {
  return prefix.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

/** The pattern for `prefixes`, or null when none is known. */
export function cardIdPattern(prefixes: readonly string[]): RegExp | null {
  const known = [...new Set(prefixes.filter((p) => p.length > 0))];
  if (known.length === 0) {
    return null;
  }
  // Longest first, so `HX-1` isn't read as `H` + `X-1`.
  // oxlint-disable-next-line unicorn/no-array-sort
  const alternatives = known.sort((a, b) => b.length - a.length).map(escape);
  return new RegExp(`(?<![\\w-])(?:${alternatives.join("|")})-\\d{1,5}(?![\\w]|-[A-Za-z])`, "g");
}

/** `text` split into plain pieces and card ids. */
export function splitCardIds(text: string, pattern: RegExp | null): Segment[] {
  if (pattern === null || text.length === 0) {
    return [{ kind: "text", text }];
  }
  const out: Segment[] = [];
  let last = 0;
  for (const match of text.matchAll(pattern)) {
    const at = match.index;
    if (at > last) {
      out.push({ kind: "text", text: text.slice(last, at) });
    }
    out.push({ kind: "id", id: match[0] });
    last = at + match[0].length;
  }
  if (last < text.length) {
    out.push({ kind: "text", text: text.slice(last) });
  }
  return out;
}

/** Whether `text` is exactly one card id: inline code linked as a whole. */
export function isCardId(text: string, pattern: RegExp | null): boolean {
  const pieces = splitCardIds(text, pattern);
  return pieces.length === 1 && pieces[0].kind === "id";
}
