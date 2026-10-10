// The owner's hand on cleanups from the dashboard (H-275): the held-cleanup
// choice and Clean up. Each reply is followed by a fresh read; a refusal is a
// toast, in the daemon's words ("Do it on mac or your phone.").

import { createElement, useCallback, useState } from "react";
import type { ReactElement } from "react";
import type { AddToast } from "../../app/useToasts";
import type { DaemonApi } from "../../protocol/api";
import { sizeText } from "../../protocol/cleanup";
import type { AttentionRowJson } from "../../protocol/dashboard";
import { errText } from "../../util";
import CleanupDialog from "./CleanupDialog";
import type { CleanupRowActions } from "./cleanupRows";

export interface CleanupState {
  /** The held cleanup the owner is choosing for. */
  readonly deciding: AttentionRowJson | null;
  readonly busy: boolean;
  /** The computer a Clean up is running on. */
  readonly cleaningUp: string | null;
  readonly decide: (row: AttentionRowJson) => void;
  readonly cancel: () => void;
  readonly resolve: (action: "remove" | "keep") => Promise<void>;
  readonly cleanUp: (machine: string) => Promise<void>;
}

export function useCleanup(
  client: DaemonApi,
  projectId: string,
  addToast: AddToast,
  refresh: () => Promise<void>,
): CleanupState {
  const [deciding, setDeciding] = useState<AttentionRowJson | null>(null);
  const [busy, setBusy] = useState(false);
  const [cleaningUp, setCleaningUp] = useState<string | null>(null);
  const cancel = useCallback(() => setDeciding(null), []);
  const resolve = useCallback(
    async (action: "remove" | "keep"): Promise<void> => {
      const job = deciding?.cleanup_job_id;
      if (job === undefined) {
        return;
      }
      setBusy(true);
      try {
        const reply = await client.request(
          { type: "cleanup_resolve", project_id: projectId, job_id: job, action },
          "cleanup_item",
        );
        const item = reply.cleanup_item;
        if (action === "keep") {
          addToast("info", "Kept", "The worktree stays; you won't be asked about it again.");
        } else if (item.state === "done") {
          addToast(
            "info",
            "Removed",
            `Freed ${sizeText(item.freed_bytes)}. Its changes are in the salvage folder.`,
          );
        } else {
          addToast("info", "Removing…", `${item.machine} removes it; the row goes once it has.`);
        }
        setDeciding(null);
      } catch (failure) {
        addToast("error", "Couldn't do that", errText(failure));
      }
      setBusy(false);
      await refresh();
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
  return { deciding, busy, cleaningUp, decide: setDeciding, cancel, resolve, cleanUp };
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
    title: c.deciding.title,
    busy: c.busy,
    onRemove: () => void c.resolve("remove"),
    onKeep: () => void c.resolve("keep"),
    onCancel: c.cancel,
  });
}
