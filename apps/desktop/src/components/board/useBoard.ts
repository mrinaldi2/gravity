import { useCallback, useEffect, useRef, useState } from "react";
import type { BoardApi } from "../../protocol/board";
import type { BoardStatus } from "./boardSync";
import { BoardSync } from "./boardSync";

interface UseBoard {
  readonly status: BoardStatus;
  /** Refetches the snapshot, or retries the watch after a failure. */
  readonly refresh: () => void;
}

/**
 * One project's live board. The watch is (re)opened whenever `live` turns
 * true, because watches end with the connection (B4 sync rule 5).
 */
export function useBoard(api: BoardApi, projectId: string, live: boolean): UseBoard {
  const [status, setStatus] = useState<BoardStatus>({ kind: "loading" });
  const sync = useRef<BoardSync | null>(null);

  useEffect(() => {
    if (!live) {
      return undefined;
    }
    const current = new BoardSync(api, projectId, setStatus);
    sync.current = current;
    current.start();
    return () => {
      current.stop();
      sync.current = null;
    };
  }, [api, projectId, live]);

  const refresh = useCallback((): void => {
    sync.current?.refresh();
  }, []);

  return { status, refresh };
}
