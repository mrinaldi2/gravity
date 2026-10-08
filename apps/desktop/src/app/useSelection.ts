import { useCallback, useRef, useState } from "react";
import type { MutableRefObject } from "react";
import { loadProjectTab, saveProjectTab } from "../settings";
import type { History } from "./history";
import {
  captureScroll,
  currentEntry,
  pushHistory,
  restoreScroll,
  startHistory,
  stepHistory,
  updateCurrent,
} from "./history";
import type { Selection } from "./selection";
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
  /** Returns to the previous view (-1) or goes forward again (1), as it was left. */
  readonly go: (direction: -1 | 1) => void;
  /** Records a move inside the view (a bot page's tab), so going back returns to it. */
  readonly note: (selection: Selection) => void;
}

/** Fills in a project window's tab: the one it last showed, else the Overview. */
function withProjectTab(selection: Selection): Selection {
  if (selection.kind !== "project" || selection.tab !== undefined) {
    return selection;
  }
  return { ...selection, tab: loadProjectTab(selection.projectId) ?? "overview" };
}

/** The pane whose scroll history keeps. */
function mainPane(): Element | null {
  return document.querySelector("main.main");
}

/**
 * The current selection and the ways it changes: an explicit pick or a bot
 * opened by id. The app opens on the projects home (UX-024). Selecting always
 * clears the target's unread badge, so the two stay in step here rather than at
 * each call site. Every pick goes into the app history (UX-035 §5).
 */
export function useSelection(unread: UnreadApi): SelectionApi {
  const [selection, setSelection] = useState<Selection>({ kind: "home" });
  const selectionRef = useLatestRef(selection);
  const history = useRef<History>(startHistory(selection));
  const restoring = useRef<() => void>(() => undefined);

  const show = useCallback(
    (next: Selection): void => {
      setSelection(next);
      unread.clearFor(next);
      if (next.kind === "project" && next.tab !== undefined) {
        saveProjectTab(next.projectId, next.tab);
      }
    },
    [unread],
  );

  /** Keeps where the view being left was scrolled. */
  const leave = useCallback((): void => {
    restoring.current();
    history.current = updateCurrent(history.current, { scroll: captureScroll(mainPane()) });
  }, []);

  const select = useCallback(
    (requested: Selection): void => {
      const next = withProjectTab(requested);
      leave();
      history.current = pushHistory(history.current, next);
      show(next);
    },
    [leave, show],
  );

  const go = useCallback(
    (direction: -1 | 1): void => {
      const moved = stepHistory(history.current, direction);
      if (moved === null) {
        return;
      }
      leave();
      history.current = moved;
      const entry = currentEntry(moved);
      show(entry.selection);
      restoring.current = restoreScroll(mainPane, entry.scroll);
    },
    [leave, show],
  );

  const note = useCallback((next: Selection): void => {
    history.current = updateCurrent(history.current, { selection: next });
  }, []);

  const selectBot = useCallback(
    (botId: string): void => {
      select({ kind: "bot", botId });
    },
    [select],
  );

  return { selection, selectionRef, select, selectBot, go, note };
}
