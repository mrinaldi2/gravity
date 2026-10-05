// The project's Dashboard tab (H-018 §2, U5): act first (Needs you), then
// state (board, releases, team, meetings), on one scrolling page. A
// release's review opens in a drawer, the same one Releases and Decisions
// show, so there is one way to rule on a package.

import { RefreshCw, X } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import type { ReactElement } from "react";
import type { ProjectTab } from "../../app/selection";
import type { AddToast } from "../../app/useToasts";
import type { DaemonApi } from "../../protocol/api";
import type { Bot, Project } from "../../protocol/entities";
import type { Release } from "../../protocol/releases";
import ReleaseReview from "../releases/ReleaseReview";
import { botNamer, useItemTitles } from "../releases/ReleasesView";
import { useReleaseActions } from "../releases/useReleases";
import NeedsYou from "./NeedsYou";
import { useDashboard } from "./useDashboard";
import { BoardWidget, MeetingsWidgets, ReleasesWidget, TeamWidget } from "./Widgets";

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
  const drawer = useRef<HTMLElement>(null);
  const close = useRef<HTMLButtonElement>(null);
  const { onClose } = props;
  // Keyboard users land in the drawer, and Escape leaves it unless one of
  // the review's own dialogs is open: that Escape is the dialog's (UX-010).
  useEffect(() => {
    close.current?.focus();
    const onKeyDown = (event: KeyboardEvent): void => {
      if (event.key === "Escape" && !drawer.current?.querySelector('[role="dialog"]')) {
        onClose();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [onClose]);
  return (
    <aside ref={drawer} className="dash-drawer" aria-label={`Release ${release.name}`}>
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
      />
    </aside>
  );
}

export default function DashboardView(props: DashboardViewProps): ReactElement {
  const { client, project, bots, connected } = props;
  const { dashboard, error, refresh } = useDashboard(client, project.id, connected);
  const [reviewing, setReviewing] = useState<Release | null>(null);
  // The Review button that opened the drawer gets focus back when it closes.
  const opener = useRef<HTMLElement | null>(null);
  const openReview = useCallback((release: Release): void => {
    opener.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    setReviewing(release);
  }, []);
  const closeReview = useCallback((): void => {
    setReviewing(null);
    if (opener.current?.isConnected) {
      opener.current.focus();
    }
    opener.current = null;
  }, []);

  if (dashboard === null) {
    return (
      <div className="empty-pane">
        <div className="empty-state" role="status">
          {error === null ? (
            <p>Loading the dashboard…</p>
          ) : (
            <>
              <p>Couldn't load the dashboard: {error}</p>
              <button type="button" className="btn" onClick={() => void refresh()}>
                Try again
              </button>
            </>
          )}
        </div>
      </div>
    );
  }

  const botName = botNamer(bots);
  const columns = new Map(dashboard.board?.columns.map((c) => [c.key, c.name]) ?? []);
  const openBoard = (): void => props.onOpenTab("board");
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
            botName={botName}
            columnName={(key) => columns.get(key) ?? key}
            onReview={openReview}
            onDecision={props.onOpenDecision}
            onBoard={openBoard}
          />
          <BoardWidget board={dashboard.board} home={dashboard.home} onOpen={openBoard} />
          <ReleasesWidget
            releases={dashboard.releases}
            onOpen={() => props.onOpenTab("releases")}
          />
          <TeamWidget team={dashboard.team} bots={bots} onOpenBot={props.onOpenBot} />
          <MeetingsWidgets />
        </div>
      </div>
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
          onClose={closeReview}
        />
      ) : null}
    </div>
  );
}
