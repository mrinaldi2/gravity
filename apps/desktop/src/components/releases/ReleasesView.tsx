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
import ReleaseReview from "./ReleaseReview";
import { useProjectReleases, useReleaseActions } from "./useReleases";

export interface ReleasesViewProps {
  readonly client: DaemonApi;
  readonly project: Project;
  readonly bots: readonly Bot[];
  readonly connected: boolean;
  readonly canControl: boolean;
  readonly addToast: AddToast;
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

export function botNamer(bots: readonly Bot[]): (id: string) => string {
  return (id) => bots.find((b) => b.id === id)?.name ?? "a bot";
}

export default function ReleasesView({
  client,
  project,
  bots,
  connected,
  canControl,
  addToast,
}: ReleasesViewProps): ReactElement {
  const { releases, loaded, replace } = useProjectReleases(client, connected, project.id, addToast);
  const actions = useReleaseActions(client, addToast, replace);
  const titles = useItemTitles(client, project.id, releases.length > 0);
  const [selected, setSelected] = useState<string | null>(null);
  const current = releases.filter(isCurrent);
  const history = releases.filter((r) => !isCurrent(r));
  const shown = releases.find((r) => r.id === selected) ?? current[0] ?? history[0];
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
            {r.items.length} item{r.items.length === 1 ? "" : "s"} · {label.word}
          </span>
        </button>
      </li>
    );
  };

  if (loaded && releases.length === 0) {
    return (
      <div className="empty-pane">
        <div className="empty-state">
          <h1>No release yet</h1>
          <p>DevOps packages the items in Verify; each package shows up here for you to test.</p>
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
      </nav>
      {shown ? (
        <ReleaseReview
          key={shown.id}
          release={shown}
          titles={titles}
          botName={botNamer(bots)}
          actions={actions}
          canControl={canControl}
        />
      ) : null}
    </div>
  );
}
