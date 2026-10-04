import type { ReactElement, ReactNode } from "react";
import type { ProjectTab } from "../../app/selection";
import type { Project } from "../../protocol/entities";
import ProjectTabs, { PROJECT_PANEL_ID, projectTabId } from "./ProjectTabs";
import { useProjectKeys } from "./useProjectKeys";

interface ProjectWindowProps {
  readonly project: Project;
  readonly botCount: number;
  readonly tab: ProjectTab;
  readonly onSelectTab: (tab: ProjectTab) => void;
  /** The active tab's view. */
  readonly children: ReactNode;
}

/** A project's window: its name, the six tabs, and the active tab's view. */
export default function ProjectWindow(props: ProjectWindowProps): ReactElement {
  const { project, botCount, tab, onSelectTab } = props;
  useProjectKeys(onSelectTab);

  return (
    <div className="project-window">
      <header className="project-window-header" data-tauri-drag-region="deep">
        <div className="view-header-main" data-tauri-drag-region="deep">
          <h2 className="view-title">{project.name}</h2>
          <span className="state-chip">
            {botCount} {botCount === 1 ? "bot" : "bots"}
          </span>
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
