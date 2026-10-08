// Card ids as links across the whole app (UX-035): the shared lookup and the
// card drawer over every view, with "Open on the board" opening the card's
// project on its Board; and ⌘[ / ⌘] back and forward through the views.

import type { ReactElement, ReactNode } from "react";
import type { DaemonState } from "../../app/useDaemonState";
import type { Selection } from "../../app/selection";
import { useHistoryKeys } from "../../app/useHistoryKeys";
import type { DaemonApi } from "../../protocol/api";
import type { Bot } from "../../protocol/entities";
import CardDrawer, { useCardDrawer } from "./CardDrawer";
import { CardLinksProvider } from "./CardLinks";

/** The project on screen, whose cards' previews don't name it. */
function currentProjectId(selection: Selection, bots: readonly Bot[]): string | null {
  if (selection.kind === "project") {
    return selection.projectId;
  }
  if (selection.kind === "bot") {
    return bots.find((b) => b.id === selection.botId)?.project_id ?? null;
  }
  return null;
}

export default function CardsLayer(props: {
  readonly client: DaemonApi;
  readonly daemon: DaemonState;
  readonly children: ReactNode;
}): ReactElement {
  const { client, daemon } = props;
  const cards = useCardDrawer();
  // Back and forward through the views; the open drawer takes them first.
  useHistoryKeys(daemon.go);
  return (
    <CardLinksProvider
      client={client}
      projects={daemon.projects}
      bots={daemon.bots}
      currentProjectId={currentProjectId(daemon.selection, daemon.bots)}
      onOpen={cards.open}
    >
      {props.children}
      <CardDrawer
        drawer={cards}
        client={client}
        bots={daemon.bots}
        canComment={daemon.canControl}
        onOpenBoard={(card) =>
          daemon.select({ kind: "project", projectId: card.projectId, tab: "board", item: card.id })
        }
      />
    </CardLinksProvider>
  );
}
