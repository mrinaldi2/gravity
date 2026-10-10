// The owner's hand on cleanups from the dashboard (H-275, UX-055): the
// held-cleanup choice and Clean up. Each reply is followed by a fresh read.
// A refusal shows inside the choice, in the daemon's words ("Do it on mac or
// your phone."). Focus goes back to Decide… after Cancel, and to the next
// Needs you row (or its heading) once the row is answered.

import { createElement, useCallback, useState } from "react";
import type { ReactElement } from "react";
import type { AddToast } from "../../app/useToasts";
import type { DaemonApi } from "../../protocol/api";
import { sizeText } from "../../protocol/cleanup";
import type { AttentionRowJson } from "../../protocol/dashboard";
import { errText } from "../../util";
import CleanupDialog from "./CleanupDialog";
import type { CleanupRowActions } from "./cleanupRows";

interface Deciding {
  readonly row: AttentionRowJson;
  readonly opener: HTMLElement | null;
}

export interface CleanupState {
  /** The held cleanup the owner is choosing for. */
  readonly deciding: Deciding | null;
  readonly busy: boolean;
  readonly error: string | null;
  /** The computer a Clean up is running on. */
  readonly cleaningUp: string | null;
  readonly decide: (row: AttentionRowJson, opener: HTMLElement | null) => void;
  readonly cancel: () => void;
  readonly resolve: (action: "remove" | "keep") => Promise<void>;
  readonly cleanUp: (machine: string) => Promise<void>;
}

/** Where focus goes once the answered row is gone: the next row's button, else the heading. */
function afterRow(opener: HTMLElement | null): () => void {
  const next = opener?.closest("li")?.nextElementSibling?.querySelector("button") ?? null;
  const heading = opener?.closest("section")?.querySelector<HTMLElement>("h2") ?? null;
  return () => {
    if (next?.isConnected) {
      next.focus();
    } else {
      heading?.focus();
    }
  };
}

function toastFor(
  addToast: AddToast,
  action: "remove" | "keep",
  state: string,
  machine: string,
  freed: number,
): void {
  if (action === "keep") {
    addToast("info", "Kept", "The worktree stays; you won't be asked about it again.");
  } else if (state === "done") {
    addToast("info", "Removed", `Freed ${sizeText(freed)}. Its changes are in the salvage folder.`);
  } else {
    addToast("info", "Removing…", `${machine} is removing it. The row goes once it's done.`);
  }
}

export function useCleanup(
  client: DaemonApi,
  projectId: string,
  addToast: AddToast,
  refresh: () => Promise<void>,
): CleanupState {
  const [deciding, setDeciding] = useState<Deciding | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [cleaningUp, setCleaningUp] = useState<string | null>(null);
  const decide = useCallback((row: AttentionRowJson, opener: HTMLElement | null) => {
    setError(null);
    setDeciding({ row, opener });
  }, []);
  const cancel = useCallback(() => {
    deciding?.opener?.focus();
    setDeciding(null);
  }, [deciding]);
  const resolve = useCallback(
    async (action: "remove" | "keep"): Promise<void> => {
      const job = deciding?.row.cleanup_job_id;
      if (deciding === null || job === undefined) {
        return;
      }
      const focusNext = afterRow(deciding.opener);
      setBusy(true);
      try {
        const reply = await client.request(
          { type: "cleanup_resolve", project_id: projectId, job_id: job, action },
          "cleanup_item",
        );
        const item = reply.cleanup_item;
        toastFor(addToast, action, item.state, item.machine, item.freed_bytes);
        setDeciding(null);
        setBusy(false);
        await refresh();
        focusNext();
      } catch (failure) {
        setError(errText(failure));
        addToast(
          "error",
          action === "keep" ? "Couldn't keep the worktree" : "Couldn't remove the worktree",
          errText(failure),
        );
        setBusy(false);
      }
    },
    [client, projectId, deciding, addToast, refresh],
  );
  const cleanUp = useCallback(
    async (machine: string): Promise<void> => {
      setCleaningUp(machine);
      try {
        const reply = await client.request({ type: "cleanup_now", machine }, "cleanup_done");
        if (reply.started === true) {
          addToast("info", `Cleaning up ${machine}`, "It runs there; the disk report follows.");
        } else {
          const freed = sizeText(reply.freed_bytes ?? 0);
          addToast("info", `Cleaned up ${machine}`, `Freed ${freed}.`);
        }
      } catch (failure) {
        addToast("error", `Couldn't clean up ${machine}`, errText(failure));
      }
      setCleaningUp(null);
      await refresh();
    },
    [client, addToast, refresh],
  );
  return { deciding, busy, error, cleaningUp, decide, cancel, resolve, cleanUp };
}

/** The Needs-you row actions: none while the connection is down. */
export function cleanupRowActions(c: CleanupState, connected: boolean): CleanupRowActions {
  if (!connected) {
    return {};
  }
  return {
    onDecideCleanup: c.decide,
    onCleanUp: (machine) => void c.cleanUp(machine),
    cleaningUp: c.cleaningUp,
  };
}

/** The held-cleanup choice, while the owner is making it. */
export function CleanupChoice(props: { readonly cleanup: CleanupState }): ReactElement | null {
  const c = props.cleanup;
  if (c.deciding === null) {
    return null;
  }
  return createElement(CleanupDialog, {
    row: c.deciding.row,
    busy: c.busy,
    error: c.error,
    onRemove: () => void c.resolve("remove"),
    onKeep: () => void c.resolve("keep"),
    onCancel: c.cancel,
  });
}
