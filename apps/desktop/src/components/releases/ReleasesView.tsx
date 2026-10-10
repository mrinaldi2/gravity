// The project's Releases tab (H-018 §4A.2): packages on the left, Current
// then History, and the selected one's review on the right.

import { useEffect, useState } from "react";
import type { ReactElement } from "react";
import type { AddToast } from "../../app/useToasts";
import type { DaemonApi } from "../../protocol/api";
import { boardCall } from "../../protocol/board";
import type { Bot, Project } from "../../protocol/entities";
import type { Release } from "../../protocol/releases";
import { isCurrent, releaseTitle, statusLabel } from "./labels";
import type { BotName } from "./labels";
import ReleaseReview from "./ReleaseReview";
import { previousName } from "./releaseMain";
import TestedOn from "./TestedOn";
import type { WaitingActions } from "./WaitingForYou";
import { anyInProgress, useLiveProgress } from "./useLiveProgress";
import { usePrRecords } from "./usePrRecords";
import { useProjectReleases, useReleaseActions } from "./useReleases";

export interface ReleasesViewProps {
  readonly client: DaemonApi;
  readonly project: Project;
  readonly bots: readonly Bot[];
  readonly connected: boolean;
  readonly canControl: boolean;
  readonly addToast: AddToast;
  /** Opens a decision where the owner answers it ("Waiting for you", H-247). */
  readonly onOpenDecision?: (decisionId: string) => void;
  /** Opens Needs you. */
  readonly onOpenNeedsYou?: () => void;
  /** Opens a pull request in the Pull requests tab (H-278); none, PR numbers are plain text. */
  readonly onOpenPr?: (num: number) => void;
}

/** Item titles from the board, which a package only names by id. */
export function useItemTitles(
  client: DaemonApi,
  projectId: string,
  wanted: boolean,
): ReadonlyMap<string, string> {
  const [titles, setTitles] = useState<ReadonlyMap<string, string>>(new Map());
  useEffect(() => {
    if (!wanted) {
      return;
    }
    let live = true;
    const load = async (): Promise<void> => {
      try {
        const board = await boardCall(client, { case: "boardGet", value: { projectId } }, "board");
        if (live) {
          setTitles(new Map(board.cards.map((c) => [c.id, c.title])));
        }
      } catch {
        // Titles are a courtesy: a package still reads by item id without them.
      }
    };
    void load();
    return () => {
      live = false;
    };
  }, [client, projectId, wanted]);
  return titles;
}

export function botNamer(bots: readonly Bot[]): BotName {
  return (id) => bots.find((b) => b.id === id)?.name;
}

/** A release's line changed on the service (H-247): read the releases again. */
function useReleaseUpdates(client: DaemonApi, projectId: string, reload: () => Promise<void>) {
  useEffect(
    () =>
      client.on("release_updated", (push) => {
        if (push.project_id === projectId) {
          void reload();
        }
      }),
    [client, projectId, reload],
  );
}

export default function ReleasesView(props: ReleasesViewProps): ReactElement {
  const { client, project, bots, connected, canControl, addToast } = props;
  const { releases, loaded, replace, reload } = useProjectReleases(
    client,
    connected,
    project.id,
    addToast,
  );
  useLiveProgress(client, project.id, connected && anyInProgress(releases), reload);
  useReleaseUpdates(client, project.id, reload);
  const waiting: WaitingActions = {
    client,
    connected,
    bots,
    addToast,
    onDecision: (id) => props.onOpenDecision?.(id),
    onNeedsYou: () => props.onOpenNeedsYou?.(),
  };
  const actions = useReleaseActions(client, addToast, replace);
  const titles = useItemTitles(client, project.id, releases.length > 0);
  const [selected, setSelected] = useState<string | null>(null);
  const current = releases.filter(isCurrent);
  const history = releases.filter((r) => !isCurrent(r));
  const shown = releases.find((r) => r.id === selected) ?? current[0] ?? history[0];
  const records = usePrRecords(client, project.id, shown, connected);
  const row = (r: Release): ReactElement => {
    const label = statusLabel(r.status);
    return (
      <li key={r.id}>
        <button
          type="button"
          className={r.id === shown?.id ? "release-list-row on" : "release-list-row"}
          aria-current={r.id === shown?.id ? "true" : undefined}
          onClick={() => setSelected(r.id)}
        >
          <span aria-hidden="true">{label.glyph}</span>
          <span className="release-list-name">{releaseTitle(r)}</span>
          <span className="release-meta">
            {r.status === "planned" && r.readiness
              ? `${r.readiness.items_ready}/${r.readiness.items_total} ready`
              : `${r.items.length} item${r.items.length === 1 ? "" : "s"}`}{" "}
            · {label.word}
          </span>
          {r.owner_blockers?.length ? (
            <span className="release-waits">
              <span aria-hidden="true">▲</span> Waiting for you · {r.owner_blockers.length}
            </span>
          ) : null}
        </button>
      </li>
    );
  };

  if (loaded && releases.length === 0) {
    return (
      <div className="empty-pane">
        <div className="empty-state">
          <h1>No release yet</h1>
          <p>
            The lead plans each release as soon as its contents are decided; it shows up here with
            every item&apos;s progress, then for you to test.
          </p>
        </div>
      </div>
    );
  }
  return (
    <div className="releases-view">
      <nav className="release-list" aria-label="Releases">
        <h3>Current</h3>
        {current.length ? (
          <ul>{current.map(row)}</ul>
        ) : (
          <p className="release-hint">Nothing waiting.</p>
        )}
        {history.length ? (
          <>
            <h3>History</h3>
            <ul>{history.map(row)}</ul>
          </>
        ) : null}
        <TestedOn
          client={client}
          projectId={project.id}
          connected={connected}
          canApprove={connected && client.hasGrant("approve")}
          botName={botNamer(bots)}
          addToast={addToast}
        />
      </nav>
      {shown ? (
        <ReleaseReview
          key={shown.id}
          release={shown}
          titles={titles}
          botName={botNamer(bots)}
          actions={actions}
          canControl={canControl}
          client={client}
          waiting={waiting}
          previous={previousName(shown, releases)}
          records={records}
          onOpenPr={props.onOpenPr}
        />
      ) : null}
    </div>
  );
}
