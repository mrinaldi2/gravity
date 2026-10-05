// The confirm in the dashboard's relayed-rulings dialog (H-112, UX-016):
// one `confirm_relayed` for exactly the rulings the dialog listed, then a
// toast saying what happened and a fresh read. When the daemon reports some
// as changed (ARCH-R42 M1), the dialog stays open on the reread list.

import { useState } from "react";
import type { AddToast } from "../../app/useToasts";
import type { DaemonApi } from "../../protocol/api";
import { errText } from "../../util";

/** How a confirm ended: `changed` keeps the dialog open on the new list. */
export type ConfirmOutcome = "done" | "changed" | "failed";

export interface ConfirmRelayed {
  readonly confirming: boolean;
  readonly confirm: (decisionIds: readonly string[]) => Promise<ConfirmOutcome>;
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
  const confirm = async (decisionIds: readonly string[]): Promise<ConfirmOutcome> => {
    setConfirming(true);
    let outcome: ConfirmOutcome = "failed";
    try {
      const reply = await client.request(
        { type: "confirm_relayed", project_id: projectId, decision_ids: decisionIds },
        "relayed_confirmed",
      );
      if (reply.failed.length > 0) {
        addToast(
          "warn",
          `Confirmed ${rulings(reply.confirmed.length)}, ${reply.failed.length} not`,
          reply.failed.map((f) => f.message).join("\n"),
        );
      } else if (reply.confirmed.length > 0) {
        addToast(
          "info",
          `Confirmed ${rulings(reply.confirmed.length)}`,
          "They're your own word now.",
        );
      }
      outcome = reply.changed.length > 0 ? "changed" : "done";
    } catch (failure) {
      addToast("error", "Couldn't confirm the rulings", errText(failure));
    } finally {
      setConfirming(false);
      await refresh();
    }
    return outcome;
  };
  return { confirming, confirm };
}
