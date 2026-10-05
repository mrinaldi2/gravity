// The owner actions of one place in the app (H-117 R2): an item's drawer, a
// decision, or the dashboard's Needs you. Nothing renders when there are
// none.

import type { ReactElement } from "react";
import type { AddToast } from "../../app/useToasts";
import type { DaemonApi } from "../../protocol/api";
import type { Bot } from "../../protocol/entities";
import OwnerActionCard from "./OwnerActionCard";
import { useOwnerActions } from "./useOwnerActions";
import type { OwnerActionScope } from "./useOwnerActions";

export default function OwnerActionList(props: {
  readonly client: DaemonApi;
  readonly connected: boolean;
  readonly scope: OwnerActionScope;
  readonly addToast: AddToast;
  readonly botName: (id: string) => string;
  readonly title?: string;
}): ReactElement | null {
  const { client, connected, scope, addToast } = props;
  const { actions, output, run, reject } = useOwnerActions(client, connected, scope, addToast);
  if (actions.length === 0) {
    return null;
  }
  const canRun = connected && client.hasGrant("approve");
  const proposer = (by: string): string =>
    by.startsWith("bot:") ? props.botName(by.slice(4)) : "The Hermes";
  return (
    <section className="owner-actions" aria-label={props.title ?? "Commands for you to run"}>
      {props.title ? <h3>{props.title}</h3> : null}
      {actions.map((a) => (
        <OwnerActionCard
          key={a.id}
          action={a}
          output={output[a.id]}
          proposer={proposer(a.proposed_by)}
          canRun={canRun}
          onRun={(action) => void run(action)}
          onReject={(action, reason) => void reject(action, reason)}
        />
      ))}
    </section>
  );
}

/** The open item's commands, in the board's item drawer. */
export function ItemOwnerActions(props: {
  readonly client: DaemonApi;
  readonly connected: boolean;
  readonly projectId: string;
  readonly itemId: string | null;
  readonly bots: readonly Bot[];
  readonly addToast: AddToast;
}): ReactElement | null {
  const { bots, itemId } = props;
  if (itemId === null) {
    return null;
  }
  return (
    <OwnerActionList
      client={props.client}
      connected={props.connected}
      scope={{ projectId: props.projectId, itemId }}
      addToast={props.addToast}
      botName={(id) => bots.find((b) => b.id === id)?.name ?? "a bot"}
    />
  );
}
