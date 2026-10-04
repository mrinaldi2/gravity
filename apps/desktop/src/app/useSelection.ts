import { useCallback, useState } from "react";
import type { MutableRefObject } from "react";
import type { Bot, Project } from "../protocol/entities";
import { loadProjectTab, saveLastUsedBotId, saveProjectTab } from "../settings";
import type { Selection } from "./selection";
import { useInitialBot } from "./useInitialBot";
import { useLatestRef } from "./useLatestRef";
import type { UnreadApi } from "./useUnread";

export interface SelectionApi {
  /** What the main pane is currently showing. */
  readonly selection: Selection;
  /** The rendered selection, for callbacks that fire outside React. */
  readonly selectionRef: MutableRefObject<Selection>;
  /** Opens a view and clears whatever badge it was carrying. */
  readonly select: (next: Selection) => void;
  /** Opens a bot by id. */
  readonly selectBot: (botId: string) => void;
}

/** Fills in a project window's tab: the one it last showed, else the Dashboard. */
function withProjectTab(selection: Selection): Selection {
  if (selection.kind !== "project" || selection.tab !== undefined) {
    return selection;
  }
  return { ...selection, tab: loadProjectTab(selection.projectId) ?? "dashboard" };
}

/**
 * The current selection and the ways it changes: an explicit pick, a bot opened
 * by id, and the one-off startup pick made by `useInitialBot`. Selecting always
 * clears the target's unread badge, so the two stay in step here rather than at
 * each call site.
 */
export function useSelection(
  projects: readonly Project[],
  bots: readonly Bot[],
  unread: UnreadApi,
): SelectionApi {
  const [selection, setSelection] = useState<Selection>({ kind: "none" });
  const selectionRef = useLatestRef(selection);

  const select = useCallback(
    (requested: Selection): void => {
      const next = withProjectTab(requested);
      setSelection(next);
      unread.clearFor(next);
      if (next.kind === "bot") {
        saveLastUsedBotId(next.botId);
      }
      if (next.kind === "project" && next.tab !== undefined) {
        saveProjectTab(next.projectId, next.tab);
      }
    },
    [unread],
  );

  const selectBot = useCallback(
    (botId: string): void => {
      select({ kind: "bot", botId });
    },
    [select],
  );

  useInitialBot(projects, bots, selection, select);

  return { selection, selectionRef, select, selectBot };
}
