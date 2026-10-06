import { useState } from "react";
import type { ReactElement } from "react";
import type { AddToast } from "../../app/useToasts";
import type { DaemonApi } from "../../protocol/api";
import type { Bot, Project } from "../../protocol/entities";
import type { ProjectRow } from "../../protocol/gen/hermes/home/v1/home_pb";
import { errText } from "../../util";
import { useNow } from "../control/useNow";
import NewProjectForm from "../sidebar/NewProjectForm";
import { homeSummary, ORDER_RULE } from "./homeText";
import ProjectCard from "./ProjectCard";
import { useProjectsOverview } from "./useProjectsOverview";

interface ProjectsHomeProps {
  readonly client: DaemonApi;
  readonly projects: readonly Project[];
  readonly bots: readonly Bot[];
  readonly connected: boolean;
  readonly canControl: boolean;
  readonly addToast: AddToast;
  readonly onOpenProject: (projectId: string) => void;
  readonly onCreateProject: (name: string) => Promise<void>;
  /** Pins the clock, for stories and visual baselines. */
  readonly now?: number;
}

/** "#1", "#2"… for the projects that need the owner, in the service's order. */
function places(rows: readonly ProjectRow[]): ReadonlyMap<string, number> {
  const map = new Map<string, number>();
  for (const row of rows) {
    if ((row.attention?.count ?? 0) > 0) {
      map.set(row.projectId, map.size + 1);
    }
  }
  return map;
}

/** How many computers the home draws from: the sources, else this one. */
function computerCount(rows: readonly ProjectRow[], sources: number): number {
  if (sources > 0) {
    return sources;
  }
  return new Set(rows.flatMap((row) => row.members.map((member) => member.daemonId))).size;
}

/** Where the app opens (UX-024): every project, ranked by how much it needs you. */
export default function ProjectsHome(props: ProjectsHomeProps): ReactElement {
  const { client, connected, addToast } = props;
  const overview = useProjectsOverview(client, connected, props.projects, props.bots);
  const clockNow = useNow();
  const now = props.now ?? clockNow;
  const [creating, setCreating] = useState(false);
  const ranked = places(overview.rows);

  const onPin = (projectId: string, pinned: boolean): void => {
    overview.pin(projectId, pinned).catch((failure: unknown) => {
      addToast("error", pinned ? "Couldn't pin the project" : "Couldn't unpin", errText(failure));
    });
  };

  return (
    <div className="home">
      <header className="home-head" data-tauri-drag-region="deep">
        <h2 className="home-title">Projects</h2>
        <span className="home-count">
          {homeSummary(overview.rows.length, computerCount(overview.rows, overview.sources.length))}
        </span>
        <button type="button" className="home-rule" title={ORDER_RULE}>
          Why this order?
        </button>
        <span className="home-spacer" />
        {props.canControl ? (
          <button
            type="button"
            className="btn btn-small"
            onClick={() => {
              setCreating(true);
            }}
          >
            ＋ New project
          </button>
        ) : null}
      </header>
      <div className="home-scroll">
        {creating ? (
          <div className="home-new">
            <NewProjectForm
              onCreate={props.onCreateProject}
              onClose={() => {
                setCreating(false);
              }}
            />
          </div>
        ) : null}
        {overview.error !== null && !overview.loaded ? (
          <p className="home-error" role="alert">
            Couldn't load the projects: {overview.error}
          </p>
        ) : null}
        {overview.legacy && overview.rows.length > 0 ? (
          <p className="home-legacy">
            This computer runs an older Hermes, so projects show without their counts. Update it to
            see what needs you.
          </p>
        ) : null}
        <div className="home-grid">
          {overview.rows.map((row) => (
            <ProjectCard
              key={row.projectId}
              row={row}
              sources={overview.sources}
              place={ranked.get(row.projectId)}
              now={now}
              canPin={props.canControl}
              onOpen={props.onOpenProject}
              onPin={onPin}
            />
          ))}
        </div>
      </div>
    </div>
  );
}
