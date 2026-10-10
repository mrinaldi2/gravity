// The project's Dashboard tab (H-018 §2, U5): act first (Needs you), then
// state (board, releases, team, meetings), on one scrolling page. A
// release's review opens in a drawer, the same one Releases and Decisions
// show, so there is one way to rule on a package.

import { RefreshCw, X } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import type { ReactElement, ReactNode } from "react";
import type { ProjectTab } from "../../app/selection";
import type { AddToast } from "../../app/useToasts";
import { useDrawerEscape } from "../../hooks/useDrawerEscape";
import type { DaemonApi } from "../../protocol/api";
import type { Bot, Project } from "../../protocol/entities";
import type { Release } from "../../protocol/releases";
import ReleaseReview from "../releases/ReleaseReview";
import { botNamer, useItemTitles } from "../releases/ReleasesView";
import { useReleaseActions } from "../releases/useReleases";
import OwnerActionList from "../ownerActions/OwnerActionList";
import DashboardItem from "./DashboardItem";
import DiskWidget from "./DiskWidget";
import FlowWidget from "./FlowWidget";
import NeedsYou from "./NeedsYou";
import { useConfirmRelayed } from "./useConfirmRelayed";
import { useDashboard } from "./useDashboard";
import { useDashboardDrawers } from "./useDashboardDrawers";
import { useMetrics } from "./useMetrics";
import { useNeedsYouPrs } from "./useNeedsYouPrs";
import { ActionItemsWidget, MeetingsWidget, offHomeOf } from "./MeetingWidgets";
import { useActionItems } from "./useActionItems";
import { CleanupChoice, cleanupRowActions, useCleanup } from "./useCleanup";
import { BoardWidget, ReleasesWidget, TeamWidget } from "./Widgets";

export interface DashboardViewProps {
  readonly client: DaemonApi;
  readonly project: Project;
  readonly bots: readonly Bot[];
  readonly connected: boolean;
  readonly canControl: boolean;
  readonly addToast: AddToast;
  readonly onOpenTab: (tab: ProjectTab) => void;
  readonly onOpenBot: (botId: string) => void;
  readonly onOpenDecision: (decisionId: string) => void;
  /** Opens a message to a bot quoting what it asked (UX-024 §4). */
  readonly onReply?: (botId: string, quote: string) => void;
  /** Opens Needs you, where permission prompts are answered. */
  readonly onOpenNeedsYou?: () => void;
  /** Opens a pull request, or its delta since your approval; absent without PRs. */
  readonly onOpenPr?: (pr: number, recheck: boolean) => void;
  /** "From the team", shown right after Needs you on the Overview. */
  readonly fromTheTeam?: ReactNode;
}

function asOf(at: string): string {
  return new Date(at).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
}

/** The release review in a right drawer, over the dashboard. */
function ReviewDrawer(props: {
  readonly release: Release;
  readonly client: DaemonApi;
  readonly project: Project;
  readonly bots: readonly Bot[];
  readonly canControl: boolean;
  readonly addToast: AddToast;
  readonly onChanged: () => void;
  readonly onClose: () => void;
}): ReactElement {
  const [release, setRelease] = useState(props.release);
  const actions = useReleaseActions(props.client, props.addToast, (next) => {
    setRelease(next);
    props.onChanged();
  });
  const titles = useItemTitles(props.client, props.project.id, true);
  const close = useRef<HTMLButtonElement>(null);
  // Keyboard users land in the drawer, and Escape leaves it unless a dialog
  // or menu is open: that Escape is theirs (UX-010, UX-012).
  useEffect(() => {
    close.current?.focus();
  }, []);
  useDrawerEscape(props.onClose);
  return (
    <aside className="dash-drawer" aria-label={`Release ${release.name}`}>
      <button
        ref={close}
        type="button"
        className="dash-drawer-close"
        aria-label="Close the release"
        onClick={props.onClose}
      >
        <X size={16} aria-hidden="true" />
      </button>
      <ReleaseReview
        release={release}
        titles={titles}
        botName={botNamer(props.bots)}
        actions={actions}
        canControl={props.canControl}
        client={props.client}
      />
    </aside>
  );
}

/** Before the first read: loading, or why it failed with Try again. */
function Loading(props: { readonly error: string | null; readonly onRetry: () => void }) {
  return (
    <div className="empty-pane">
      <div className="empty-state" role="status">
        {props.error === null ? (
          <p>Loading the dashboard…</p>
        ) : (
          <>
            <p>Couldn't load the dashboard: {props.error}</p>
            <button type="button" className="btn" onClick={props.onRetry}>
              Try again
            </button>
          </>
        )}
      </div>
    </div>
  );
}

export default function DashboardView(props: DashboardViewProps): ReactElement {
  const { client, project, bots, connected } = props;
  const { dashboard, error, refresh } = useDashboard(client, project.id, connected);
  const relayed = useConfirmRelayed(client, project.id, props.addToast, refresh);
  const actionItems = useActionItems(client, project.id, props.addToast, refresh);
  const { reviewing, openItem, openReview, showItem, close: closeDrawer } = useDashboardDrawers();
  const metrics = useMetrics(client, project.id, connected);
  const prs = useNeedsYouPrs(client, project.id, connected, props.onOpenPr, dashboard);
  const cleanup = useCleanup(client, project.id, props.addToast, refresh);

  if (dashboard === null) {
    return <Loading error={error} onRetry={() => void refresh()} />;
  }

  const named = botNamer(bots);
  const botName = (id: string): string => named(id) ?? "a bot";
  const columns = new Map(dashboard.board?.columns.map((c) => [c.key, c.name]) ?? []);
  const openBoard = (): void => props.onOpenTab("board");
  const offHome = offHomeOf(dashboard);
  const leadName = named(project.lead_bot_id ?? "") ?? null;
  return (
    <div className="dash">
      <div className="dash-scroll">
        <p className="dash-asof">
          as of {asOf(dashboard.as_of)}
          <button
            type="button"
            className="dash-refresh"
            aria-label="Refresh the dashboard"
            title="Refresh"
            onClick={() => void refresh()}
          >
            <RefreshCw size={12} aria-hidden="true" />
          </button>
        </p>
        <div className="dash-grid">
          <NeedsYou
            projectName={project.name}
            rows={dashboard.needs_you}
            count={dashboard.needs_you_count}
            overrides={dashboard.wip_overrides ?? []}
            note={dashboard.needs_you_note}
            canApprove={connected && client.hasGrant("approve")}
            confirming={relayed.confirming}
            botName={(id) => botName(id) ?? "a bot"}
            columnName={(key) => columns.get(key) ?? key}
            onReview={openReview}
            onDecision={props.onOpenDecision}
            onConfirmRelayed={relayed.confirm}
            onItem={showItem}
            onOpenBot={props.onOpenBot}
            onReply={props.onReply}
            onOpenNeedsYou={props.onOpenNeedsYou}
            prs={prs}
            onOpenPr={props.onOpenPr}
            {...cleanupRowActions(cleanup, connected)}
          />
          {props.fromTheTeam}
          <OwnerActionList
            client={client}
            connected={connected}
            scope={{ projectId: project.id, waitingOnly: true }}
            addToast={props.addToast}
            botName={botName}
            title="Commands for you to run"
          />
          <BoardWidget board={dashboard.board} home={dashboard.home} onOpen={openBoard} />
          <ReleasesWidget
            releases={dashboard.releases}
            onOpen={() => props.onOpenTab("releases")}
          />
          <TeamWidget team={dashboard.team} bots={bots} onOpenBot={props.onOpenBot} />
          <MeetingsWidget rows={dashboard.meetings} leadName={leadName} offHome={offHome} />
          <ActionItemsWidget
            actions={dashboard.action_items}
            offHome={offHome}
            botName={botName}
            canControl={connected && props.canControl}
            onDone={(id) => void actionItems.setStatus(id, "done")}
            onDrop={(id) => void actionItems.setStatus(id, "dropped")}
            onPromote={(id) => void actionItems.promote(id)}
            onItem={showItem}
          />
          <FlowWidget
            state={metrics}
            columnName={(key) => columns.get(key) ?? key}
            onBoard={openBoard}
          />
          <DiskWidget
            client={client}
            connected={connected}
            onCleanUp={cleanupRowActions(cleanup, connected).onCleanUp}
            cleaningUp={cleanup.cleaningUp}
          />
        </div>
      </div>
      <CleanupChoice cleanup={cleanup} />
      {reviewing ? (
        <ReviewDrawer
          key={reviewing.id}
          release={reviewing}
          client={client}
          project={project}
          bots={bots}
          canControl={props.canControl}
          addToast={props.addToast}
          onChanged={() => void refresh()}
          onClose={closeDrawer}
        />
      ) : null}
      {openItem ? (
        <DashboardItem
          key={openItem}
          api={client}
          projectId={project.id}
          itemId={openItem}
          bots={bots}
          canComment={props.canControl}
          onClose={closeDrawer}
        >
          <OwnerActionList
            client={client}
            connected={connected}
            scope={{ projectId: project.id, itemId: openItem }}
            addToast={props.addToast}
            botName={botName}
          />
        </DashboardItem>
      ) : null}
    </div>
  );
}
