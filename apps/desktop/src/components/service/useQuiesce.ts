// The pause for an install (H-117) as the app follows it: the open pause,
// read on connect and kept current by its pushes, and one notice for how
// the last one ended, even when the install's restart dropped the push.

import { useCallback, useEffect, useState } from "react";
import type { AddToast } from "../../app/useToasts";
import { useLoadOnConnect } from "../../hooks/useLoadOnConnect";
import { notifyNatively } from "../../notify";
import type { DaemonApi } from "../../protocol/api";
import type { Quiesce } from "../../protocol/quiesce";
import { readStored, writeStored } from "../../storage";

const TOLD_KEY = "quiesce.told";

/** "0.17.0": what is being installed, in the owner's words. */
export function installing(q: Quiesce): string {
  return q.version ?? q.reason.replace(/^install of /u, "");
}

/** How a pause ended, in one sentence per outcome (UX on H-117). */
function outcomeLine(q: Quiesce): string | null {
  const resumed = q.report?.resumed;
  const what = installing(q);
  switch (resumed?.outcome) {
    case "install_ok":
      return `The Hermes ${what} is installed. Everything is running again.`;
    case "rolled_back":
      return `The install of ${what} didn't work, so this computer went back to ${resumed.running_version ?? "the previous version"}. Everything is running again.`;
    case "deadline": {
      const minutes = Math.round((Date.parse(q.deadline_at) - Date.parse(q.started_at)) / 60_000);
      return `The install of ${what} took longer than ${minutes} minutes, so everything resumed. The release has the details.`;
    }
    case "aborted":
      return `The install of ${what} stopped before it began. Everything is running again.`;
    case "resumed":
      return `You resumed everything. The install of ${what} may not finish.`;
    default:
      return null;
  }
}

/** Whether the owner already heard how pause `id` ended, here. */
function told(id: string): boolean {
  try {
    return (readStored(TOLD_KEY) ?? "").split(",").includes(id);
  } catch {
    return false;
  }
}

function markTold(id: string): void {
  try {
    const kept = (readStored(TOLD_KEY) ?? "").split(",").filter(Boolean).slice(-9);
    writeStored(TOLD_KEY, [...kept, id].join(","));
  } catch {
    // Without storage the notice may repeat after a reload; nothing else.
  }
}

export function useQuiesce(
  client: DaemonApi,
  connected: boolean,
  addToast: AddToast,
): Quiesce | null {
  const [quiesce, setQuiesce] = useState<Quiesce | null>(null);
  const load = useCallback(async (): Promise<void> => {
    try {
      const reply = await client.request({ type: "quiesce_status" }, "quiesce");
      setQuiesce(reply.quiesce);
      const ended = reply.ended;
      const line = ended ? outcomeLine(ended) : null;
      if (ended && line !== null && !told(ended.id)) {
        markTold(ended.id);
        const warn = ended.report?.resumed?.outcome !== "install_ok";
        addToast(warn ? "warn" : "info", line, "");
        void notifyNatively("The Hermes", line);
      }
    } catch {
      // An older daemon has no pause to show.
      setQuiesce(null);
    }
  }, [client, addToast]);
  useLoadOnConnect(connected, load);
  useEffect(
    () =>
      client.on("quiesce_update", (push) => {
        setQuiesce(push.quiesce);
        // It just ended: read how.
        if (push.quiesce === null) {
          void load();
        }
      }),
    [client, load],
  );
  return quiesce;
}
