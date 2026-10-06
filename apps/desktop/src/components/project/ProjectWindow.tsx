import type { ReactElement, ReactNode } from "react";
import type { ProjectTab } from "../../app/selection";
import type { Project } from "../../protocol/entities";
import type { ProjectRow } from "../../protocol/gen/hermes/home/v1/home_pb";
import { releasePill } from "../home/homeText";
import ProjectTabs, { PROJECT_PANEL_ID, projectTabId } from "./ProjectTabs";
import { useProjectKeys } from "./useProjectKeys";

interface ProjectWindowProps {
  readonly project: Project;
  readonly botCount: number;
  readonly tab: ProjectTab;
  readonly onSelectTab: (tab: ProjectTab) => void;
  /** The project's overview row, for what needs the owner and its release. */
  readonly row?: ProjectRow | null;
  /** Back to the projects home; the crumb is hidden without it. */
  readonly onHome?: () => void;
  /** The active tab's view. */
  readonly children: ReactNode;
}

/**
 * "▲ 4 need you" and the release, when the service sent the project's row.
 * Each is an action (UX-027): Needs you opens the Overview, where they are
 * listed first; the release opens Releases.
 */
function Pills(props: {
  readonly row: ProjectRow;
  readonly onSelectTab: (tab: ProjectTab) => void;
}): ReactElement {
  const { row } = props;
  const needs = row.attention?.count ?? 0;
  const release = row.currentRelease === undefined ? null : releasePill(row.currentRelease);
  return (
    <>
      {needs > 0 ? (
        <button
          type="button"
          className="release-pill release-tone-you project-pill"
          onClick={() => {
            props.onSelectTab("overview");
          }}
        >
          ▲ {needs} {needs === 1 ? "needs" : "need"} you
        </button>
      ) : null}
      {release === null ? null : (
        <button
          type="button"
          className={`release-pill release-tone-${release.tone} project-pill`}
          onClick={() => {
            props.onSelectTab("releases");
          }}
        >
          {release.text}
        </button>
      )}
    </>
  );
}

/**
 * A project's window (UX-024): a crumb back to Projects, its name with what
 * needs the owner and its release, the tabs, and the active tab's view.
 */
export default function ProjectWindow(props: ProjectWindowProps): ReactElement {
  const { project, botCount, tab, onSelectTab, row } = props;
  useProjectKeys(onSelectTab);

  return (
    <div className="project-window">
      <header className="project-window-header" data-tauri-drag-region="deep">
        <div className="view-header-main" data-tauri-drag-region="deep">
          <div className="project-title">
            {props.onHome === undefined ? null : (
              <button type="button" className="project-crumb" onClick={props.onHome}>
                Projects ›
              </button>
            )}
            <h2 className="view-title">{project.name}</h2>
          </div>
          <span className="state-chip">
            {botCount} {botCount === 1 ? "bot" : "bots"}
          </span>
          {row === undefined || row === null ? null : <Pills row={row} onSelectTab={onSelectTab} />}
        </div>
        <ProjectTabs active={tab} onSelect={onSelectTab} />
      </header>
      <div
        id={PROJECT_PANEL_ID}
        className="project-window-panel"
        role="tabpanel"
        aria-labelledby={projectTabId(tab)}
      >
        {props.children}
      </div>
    </div>
  );
}
