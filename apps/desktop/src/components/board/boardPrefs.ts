import { readStored, writeStored } from "../../storage";
import type { BoardFilters } from "./filters";
import { NO_FILTERS, parseFilters } from "./filters";

/** How the owner left a project's board: remembered per project (H-018 §3.1, §3.5). */
export interface BoardPrefs {
  readonly filters: BoardFilters;
  /** Inbox starts as a collapsed rail. */
  readonly inboxOpen: boolean;
  /** Hidden columns, such as Cancelled. */
  readonly showHidden: boolean;
}

const DEFAULT_PREFS: BoardPrefs = { filters: NO_FILTERS, inboxOpen: false, showHidden: false };
const KEY = "board.";

export function loadBoardPrefs(projectId: string): BoardPrefs {
  try {
    const raw = readStored(KEY + projectId);
    if (raw === null) {
      return DEFAULT_PREFS;
    }
    const parsed: unknown = JSON.parse(raw);
    if (typeof parsed !== "object" || parsed === null) {
      return DEFAULT_PREFS;
    }
    return {
      filters: parseFilters(Reflect.get(parsed, "filters")),
      inboxOpen: Reflect.get(parsed, "inboxOpen") === true,
      showHidden: Reflect.get(parsed, "showHidden") === true,
    };
  } catch {
    return DEFAULT_PREFS;
  }
}

export function saveBoardPrefs(projectId: string, prefs: BoardPrefs): void {
  try {
    writeStored(KEY + projectId, JSON.stringify(prefs));
  } catch {
    // localStorage unavailable; the board resets next launch
  }
}
