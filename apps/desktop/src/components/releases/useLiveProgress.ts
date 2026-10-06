// Keeps the releases' item status live (H-137): while a package isn't
// submitted, the board is watched and the releases are read again shortly
// after each change, so a card moving shows in the Releases tab.

import { useEffect } from "react";
import type { BoardApi } from "../../protocol/board";
import type { Release } from "../../protocol/releases";
import { useBoard } from "../board/useBoard";

/** How long after a board change the releases are read again. */
const SETTLE_MS = 600;

/** Whether any package still shows its items' progress. */
export function anyInProgress(releases: readonly Release[]): boolean {
  return releases.some((r) => ["planned", "assembling", "built"].includes(r.status));
}

export function useLiveProgress(
  api: BoardApi,
  projectId: string,
  live: boolean,
  reload: () => Promise<void>,
): void {
  const { status } = useBoard(api, projectId, live);
  const seq = status.kind === "ready" ? status.model.seq : null;
  useEffect(() => {
    if (seq === null) {
      return undefined;
    }
    const timer = setTimeout(() => void reload(), SETTLE_MS);
    return () => clearTimeout(timer);
  }, [seq, reload]);
}
