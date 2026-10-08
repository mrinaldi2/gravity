// App history (UX-035 §5): every view the owner opens, up to 50, with where
// its scrollable panes were, so ⌘[ / ⌘] / the mouse's back button return to
// a view as it was left.

import type { Selection } from "./selection";

/** How many views history keeps. */
export const HISTORY_LIMIT = 50;

/** One scrolled pane: its path from the main pane, and how far down it was. */
export interface ScrollMark {
  readonly path: readonly number[];
  readonly top: number;
}

export interface HistoryEntry {
  readonly selection: Selection;
  readonly scroll: readonly ScrollMark[];
}

export interface History {
  readonly entries: readonly HistoryEntry[];
  readonly index: number;
}

export function startHistory(selection: Selection): History {
  return { entries: [{ selection, scroll: [] }], index: 0 };
}

/** Opens `selection` after the current view: what was ahead is dropped. */
export function pushHistory(history: History, selection: Selection): History {
  const kept = history.entries.slice(0, history.index + 1);
  const entries = [...kept, { selection, scroll: [] }].slice(-HISTORY_LIMIT);
  return { entries, index: entries.length - 1 };
}

/** The current entry with `change` applied: its scroll, or a tab it moved to. */
export function updateCurrent(history: History, change: Partial<HistoryEntry>): History {
  const entries = history.entries.map((entry, i) =>
    i === history.index ? { ...entry, ...change } : entry,
  );
  return { entries, index: history.index };
}

/** One step back (-1) or forward (1), or `null` at either end. */
export function stepHistory(history: History, direction: -1 | 1): History | null {
  const index = history.index + direction;
  return index < 0 || index >= history.entries.length ? null : { ...history, index };
}

export function currentEntry(history: History): HistoryEntry {
  // The index always points into entries.
  return history.entries[history.index] as HistoryEntry;
}

function pathOf(root: Element, el: Element): number[] {
  const path: number[] = [];
  let node: Element | null = el;
  while (node !== null && node !== root) {
    const parent: Element | null = node.parentElement;
    if (parent === null) {
      return [];
    }
    path.unshift(Array.prototype.indexOf.call(parent.children, node));
    node = parent;
  }
  return path;
}

function atPath(root: Element, path: readonly number[]): Element | null {
  let node: Element | undefined = root;
  for (const i of path) {
    node = node?.children[i];
  }
  return node ?? null;
}

/** Where every scrolled pane under `root` is. */
export function captureScroll(root: Element | null): ScrollMark[] {
  if (root === null) {
    return [];
  }
  return Array.from(root.querySelectorAll("*"))
    .filter((el) => el.scrollTop > 0)
    .map((el) => ({ path: pathOf(root, el), top: el.scrollTop }));
}

/** How long a restore waits for the view's content to load. */
const RESTORE_MS = 2000;

/**
 * Puts each pane back where it was once its content is tall enough; at the
 * deadline whatever is still short goes as far as it can (the nearest offset).
 * Returns a cancel.
 */
export function restoreScroll(
  root: () => Element | null,
  marks: readonly ScrollMark[],
  now: () => number = Date.now,
): () => void {
  let left = [...marks];
  const deadline = now() + RESTORE_MS;
  let frame = 0;
  const step = (): void => {
    const base = root();
    const late = now() >= deadline;
    left = left.filter((mark) => {
      const el = base === null ? null : atPath(base, mark.path);
      if (el === null || (!late && el.scrollHeight - el.clientHeight < mark.top)) {
        return !late;
      }
      el.scrollTop = mark.top;
      return false;
    });
    if (left.length > 0) {
      frame = requestAnimationFrame(step);
    }
  };
  if (left.length > 0) {
    frame = requestAnimationFrame(step);
  }
  return () => cancelAnimationFrame(frame);
}
