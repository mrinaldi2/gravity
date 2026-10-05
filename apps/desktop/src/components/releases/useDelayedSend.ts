import { useCallback, useEffect, useRef, useState } from "react";
import type { AddToast } from "../../app/useToasts";

/** How long a ruling waits for Undo before it is sent (H-020 §6.7). */
export const UNDO_MS = 5000;

export interface DelayedSend {
  /** The label of the action waiting to be sent, if one is. */
  readonly pending: string | null;
  /** Wait `UNDO_MS`, offering Undo, then run `send`. */
  readonly schedule: (label: string, send: () => Promise<void>) => void;
  readonly undo: () => void;
}

/**
 * `release_rule`, `release_hold` and `release_pause` act at once on the
 * daemon, so the review's Undo is a client-side delay: the request leaves
 * only when the toast's five seconds have passed without Undo. Leaving the
 * view sends what is waiting rather than dropping it.
 */
export function useDelayedSend(addToast: AddToast): DelayedSend {
  const [pending, setPending] = useState<string | null>(null);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const waiting = useRef<(() => Promise<void>) | null>(null);

  const fire = useCallback((): void => {
    const send = waiting.current;
    waiting.current = null;
    timer.current = null;
    setPending(null);
    if (send) {
      void send();
    }
  }, []);

  const undo = useCallback((): void => {
    if (timer.current) {
      clearTimeout(timer.current);
    }
    timer.current = null;
    waiting.current = null;
    setPending(null);
  }, []);

  const schedule = useCallback(
    (label: string, send: () => Promise<void>): void => {
      if (timer.current) {
        // One at a time: what was waiting goes now.
        clearTimeout(timer.current);
        fire();
      }
      waiting.current = send;
      setPending(label);
      timer.current = setTimeout(fire, UNDO_MS);
      addToast("info", `${label}…`, "Sends in 5 seconds.", {
        action: { label: "Undo", run: undo },
      });
    },
    [addToast, fire, undo],
  );

  useEffect(
    () => () => {
      if (timer.current) {
        clearTimeout(timer.current);
        fire();
      }
    },
    [fire],
  );

  return { pending, schedule, undo };
}
