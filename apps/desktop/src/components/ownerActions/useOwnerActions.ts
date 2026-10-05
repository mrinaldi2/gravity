// Owner actions for one place in the app (H-117 R2): a project's, an item's
// or a decision's, kept current by the daemon's pushes, with each running
// action's output as it streams in.

import { useCallback, useEffect, useState } from "react";
import type { AddToast } from "../../app/useToasts";
import { useLoadOnConnect } from "../../hooks/useLoadOnConnect";
import type { DaemonApi } from "../../protocol/api";
import type { OwnerAction } from "../../protocol/ownerActions";
import { errText } from "../../util";

export interface OwnerActionScope {
  readonly projectId: string;
  readonly itemId?: string;
  readonly decisionId?: string;
  /** Only those waiting for the owner. */
  readonly waitingOnly?: boolean;
}

export interface OwnerActions {
  readonly actions: readonly OwnerAction[];
  /** Output streamed so far, by action id. */
  readonly output: Readonly<Record<string, string>>;
  readonly run: (action: OwnerAction) => Promise<void>;
  readonly reject: (action: OwnerAction, reason: string) => Promise<void>;
}

function inScope(a: OwnerAction, scope: OwnerActionScope): boolean {
  return (
    a.project_id === scope.projectId &&
    (scope.itemId === undefined || a.item_id === scope.itemId) &&
    (scope.decisionId === undefined || a.decision_id === scope.decisionId) &&
    (scope.waitingOnly !== true || a.state === "proposed" || a.state === "running")
  );
}

function upsert(list: readonly OwnerAction[], action: OwnerAction): OwnerAction[] {
  const rest = list.filter((a) => a.id !== action.id);
  return [action, ...rest];
}

export function useOwnerActions(
  client: DaemonApi,
  connected: boolean,
  scope: OwnerActionScope,
  addToast: AddToast,
): OwnerActions {
  const [all, setAll] = useState<readonly OwnerAction[]>([]);
  const [output, setOutput] = useState<Record<string, string>>({});
  const load = useCallback(async (): Promise<void> => {
    try {
      const reply = await client.request(
        { type: "owner_action_list", project_id: scope.projectId },
        "owner_actions",
      );
      setAll(reply.actions);
    } catch {
      // An older daemon has no owner actions.
      setAll([]);
    }
  }, [client, scope.projectId]);
  useLoadOnConnect(connected, load);
  useEffect(() => {
    const offs = [
      client.on("owner_action_update", (push) => setAll((list) => upsert(list, push.action))),
      client.on("owner_action_output", (push) =>
        setOutput((now) => ({ ...now, [push.id]: (now[push.id] ?? "") + push.chunk })),
      ),
    ];
    return () => {
      for (const off of offs) {
        off();
      }
    };
  }, [client]);

  const run = useCallback(
    async (action: OwnerAction): Promise<void> => {
      try {
        const reply = await client.request(
          { type: "owner_action_run", id: action.id, sha256: action.sha256 },
          "owner_action",
        );
        setAll((list) => upsert(list, reply.action));
      } catch (failure) {
        addToast("error", "Couldn't run the action", errText(failure));
      }
    },
    [client, addToast],
  );
  const reject = useCallback(
    async (action: OwnerAction, reason: string): Promise<void> => {
      try {
        const reply = await client.request(
          { type: "owner_action_reject", id: action.id, reason: reason || undefined },
          "owner_action",
        );
        setAll((list) => upsert(list, reply.action));
      } catch (failure) {
        addToast("error", "Couldn't reject the action", errText(failure));
      }
    },
    [client, addToast],
  );
  const actions = all.filter((a) => inScope(a, scope));
  // oxlint-disable-next-line unicorn/no-array-sort -- a fresh array from filter
  actions.sort(byUrgency);
  return { actions, output, run, reject };
}

function rank(x: OwnerAction): number {
  if (x.state === "running") {
    return 0;
  }
  return x.state === "proposed" ? 1 : 2;
}

/** Running first, then waiting oldest first, then the rest newest first. */
function byUrgency(a: OwnerAction, b: OwnerAction): number {
  if (rank(a) !== rank(b)) {
    return rank(a) - rank(b);
  }
  const order = a.created_at.localeCompare(b.created_at);
  return rank(a) === 1 ? order : -order;
}
