// "Confirm all" on the dashboard's relayed-rulings row (H-112): one
// `confirm_relayed` for the project, then a toast saying what happened and a
// fresh read.

import { useState } from "react";
import type { AddToast } from "../../app/useToasts";
import type { DaemonApi } from "../../protocol/api";
import { errText } from "../../util";

export interface ConfirmRelayed {
  readonly confirming: boolean;
  readonly confirm: () => void;
}

function rulings(n: number): string {
  return `${n} ruling${n === 1 ? "" : "s"}`;
}

export function useConfirmRelayed(
  client: DaemonApi,
  projectId: string,
  addToast: AddToast,
  refresh: () => Promise<void>,
): ConfirmRelayed {
  const [confirming, setConfirming] = useState(false);
  const run = async (): Promise<void> => {
    setConfirming(true);
    try {
      const reply = await client.request(
        { type: "confirm_relayed", project_id: projectId },
        "relayed_confirmed",
      );
      if (reply.failed.length === 0) {
        addToast(
          "info",
          `Confirmed ${rulings(reply.confirmed.length)}`,
          "They're your own word now.",
        );
      } else {
        addToast(
          "warn",
          `Confirmed ${rulings(reply.confirmed.length)}, ${reply.failed.length} not`,
          reply.failed.map((f) => f.message).join("\n"),
        );
      }
    } catch (failure) {
      addToast("error", "Couldn't confirm the rulings", errText(failure));
    } finally {
      setConfirming(false);
      await refresh();
    }
  };
  return { confirming, confirm: () => void run() };
}
