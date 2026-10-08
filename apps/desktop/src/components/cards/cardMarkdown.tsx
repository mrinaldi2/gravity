// Card ids in Markdown a bot or the owner wrote (UX-035 §1, §7): ids in text
// become card links; code blocks and links are left alone, and inline code
// is linked only when the whole span is an id. Every other link stays inert
// text (`SAFE_LINKS`): bot output is untrusted.

import { useMemo } from "react";
import type { ReactElement, ReactNode } from "react";
import { defaultUrlTransform } from "react-markdown";
import { isCardId, splitCardIds } from "./cardIds";
import CardLink from "./CardLink";
import { useCardLinks } from "./CardLinks";

const SCHEME = "hermes://item/";

interface MdNode {
  type: string;
  value?: string;
  url?: string;
  children?: MdNode[];
}

const SKIP = new Set(["link", "linkReference", "code", "html"]);

function cardLink(id: string, inner: MdNode): MdNode {
  return { type: "link", url: `${SCHEME}${id}`, children: [inner] };
}

function linkIds(node: MdNode, pattern: RegExp): void {
  if (node.children === undefined || SKIP.has(node.type)) {
    return;
  }
  node.children = node.children.flatMap((child): MdNode[] => {
    if (child.type === "text" && child.value !== undefined) {
      return splitCardIds(child.value, pattern).map((s) =>
        s.kind === "text"
          ? { type: "text", value: s.text }
          : cardLink(s.id, { type: "text", value: s.id }),
      );
    }
    if (
      child.type === "inlineCode" &&
      child.value !== undefined &&
      isCardId(child.value, pattern)
    ) {
      return [cardLink(child.value, child)];
    }
    linkIds(child, pattern);
    return [child];
  });
}

/** The remark plugin for `pattern`. */
function remarkCardIds(pattern: RegExp | null) {
  return () => (tree: MdNode) => {
    if (pattern !== null) {
      linkIds(tree, pattern);
    }
  };
}

function urlTransform(url: string): string {
  return url.startsWith(SCHEME) ? url : defaultUrlTransform(url);
}

type Anchor = (props: { children?: ReactNode; href?: string }) => ReactElement;

/**
 * The props a `ReactMarkdown` takes to link card ids. `inert` draws every
 * other link; a written `hermes://item/` link is a card link only when its
 * id is one, so a bot can't put other words behind it.
 */
export function useCardMarkdown(inert: Anchor) {
  const pattern = useCardLinks()?.pattern ?? null;
  return useMemo(() => {
    const a: Anchor = ({ children, href }) => {
      const id = href?.startsWith(SCHEME) ? href.slice(SCHEME.length) : null;
      return id !== null && pattern !== null && isCardId(id, pattern) ? (
        <CardLink id={id} />
      ) : (
        inert({ children, href })
      );
    };
    return { remarkPlugins: [remarkCardIds(pattern)], urlTransform, a };
  }, [pattern, inert]);
}
