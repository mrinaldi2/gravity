// The owner's hand on action items from the dashboard (H-102): tick one
// done, drop it, or promote it to a chore on the board. Each reply is
// followed by a fresh read; a refusal is a toast.

import { useCallback } from "react";
import type { AddToast } from "../../app/useToasts";
import type { DaemonApi } from "../../protocol/api";
import type { ActionStatus } from "../../protocol/meetings";
import { errText } from "../../util";

export interface ActionItemActions {
  readonly setStatus: (actionId: string, status: ActionStatus) => Promise<void>;
  readonly promote: (actionId: string) => Promise<void>;
}

export function useActionItems(
  client: DaemonApi,
  projectId: string,
  addToast: AddToast,
  refresh: () => Promise<void>,
): ActionItemActions {
  const setStatus = useCallback(
    async (actionId: string, status: ActionStatus): Promise<void> => {
      try {
        await client.request(
          { type: "action_update", project_id: projectId, action_id: actionId, status },
          "meeting_action",
        );
      } catch (failure) {
        addToast("error", "Couldn't change the action item", errText(failure));
      }
      await refresh();
    },
    [client, projectId, addToast, refresh],
  );
  const promote = useCallback(
    async (actionId: string): Promise<void> => {
      try {
        const reply = await client.request(
          { type: "action_promote", project_id: projectId, action_id: actionId },
          "meeting_action",
        );
        addToast(
          "info",
          `Promoted to ${reply.action.item_id ?? "an item"}`,
          "It's a chore in the board's Inbox, linked to its meeting.",
        );
      } catch (failure) {
        addToast("error", "Couldn't promote the action item", errText(failure));
      }
      await refresh();
    },
    [client, projectId, addToast, refresh],
  );
  return { setStatus, promote };
}
