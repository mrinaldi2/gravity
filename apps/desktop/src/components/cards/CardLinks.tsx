// What every card link in the app shares (UX-035): which prefixes are
// cards, one lookup cache, the bots' names, the project on screen, and how
// to open a card. Outside the provider an id stays plain text.

import { createContext, useContext, useEffect, useMemo, useState } from "react";
import type { ReactElement, ReactNode } from "react";
import type { DaemonApi } from "../../protocol/api";
import type { Bot, Project } from "../../protocol/entities";
import { CardCache } from "./cardCache";
import { cardIdPattern } from "./cardIds";

export interface CardLinksValue {
  readonly pattern: RegExp | null;
  readonly cache: CardCache;
  readonly botName: (id: string) => string | undefined;
  readonly currentProjectId: string | null;
  /** Opens the card in the app's drawer, read from its project's board. */
  readonly open: (id: string, projectId: string) => void;
}

const CardLinksContext = createContext<CardLinksValue | null>(null);

export function useCardLinks(): CardLinksValue | null {
  return useContext(CardLinksContext);
}

interface CardLinksProviderProps {
  readonly client: DaemonApi;
  readonly projects: readonly Project[];
  readonly bots: readonly Bot[];
  readonly currentProjectId: string | null;
  readonly onOpen: (id: string, projectId: string) => void;
  readonly children: ReactNode;
}

export function CardLinksProvider(props: CardLinksProviderProps): ReactElement {
  const { client, projects, bots, currentProjectId, onOpen } = props;
  const [cache] = useState(() => new CardCache(client));
  // A changed card is read again on its next hover.
  useEffect(() => client.onBoardEvent((event) => cache.invalidate(event.itemId)), [client, cache]);
  const prefixes = projects
    .map((p) => p.item_prefix)
    .filter((p): p is string => typeof p === "string" && p.length > 0);
  // oxlint-disable-next-line unicorn/no-array-sort -- sorts a fresh copy
  const key = [...new Set(prefixes)].sort().join(",");
  // oxlint-disable-next-line react/exhaustive-effect-dependencies -- `key` stands for `prefixes`
  const pattern = useMemo(() => cardIdPattern(key ? key.split(",") : []), [key]);
  const value = useMemo<CardLinksValue>(
    () => ({
      pattern,
      cache,
      botName: (id) => bots.find((b) => b.id === id)?.name,
      currentProjectId,
      open: onOpen,
    }),
    [pattern, cache, bots, currentProjectId, onOpen],
  );
  return <CardLinksContext.Provider value={value}>{props.children}</CardLinksContext.Provider>;
}
