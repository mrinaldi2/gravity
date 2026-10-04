import { useRef } from "react";
import type { KeyboardEvent, ReactElement } from "react";
import { PROJECT_TABS } from "../../app/selection";
import type { ProjectTab } from "../../app/selection";

export const PROJECT_TAB_LABEL: Readonly<Record<ProjectTab, string>> = {
  dashboard: "Dashboard",
  board: "Board",
  releases: "Releases",
  meetings: "Meetings",
  conversations: "Conversations",
  settings: "Settings",
};

/** The id of a tab's button, which its panel names as its label. */
export function projectTabId(tab: ProjectTab): string {
  return `project-tab-${tab}`;
}

export const PROJECT_PANEL_ID = "project-panel";

interface ProjectTabsProps {
  readonly active: ProjectTab;
  readonly onSelect: (tab: ProjectTab) => void;
}

/** Where an arrow, Home or End key moves focus from tab `index`. */
function targetIndex(key: string, index: number): number | null {
  const last = PROJECT_TABS.length - 1;
  switch (key) {
    case "ArrowRight":
      return index === last ? 0 : index + 1;
    case "ArrowLeft":
      return index === 0 ? last : index - 1;
    case "Home":
      return 0;
    case "End":
      return last;
    default:
      return null;
  }
}

/**
 * The project window's tab bar: a WAI-ARIA tablist with one tab stop, where
 * the arrow keys, Home and End move between tabs and select as they go.
 */
export default function ProjectTabs({ active, onSelect }: ProjectTabsProps): ReactElement {
  const buttons = useRef<(HTMLButtonElement | null)[]>([]);

  const onKeyDown = (event: KeyboardEvent<HTMLButtonElement>, index: number): void => {
    const next = targetIndex(event.key, index);
    const tab = next === null ? undefined : PROJECT_TABS[next];
    if (next === null || tab === undefined) {
      return;
    }
    event.preventDefault();
    onSelect(tab);
    buttons.current[next]?.focus();
  };

  return (
    <div className="tabs project-tabs" role="tablist" aria-label="Project">
      {PROJECT_TABS.map((tab, index) => {
        const selected = tab === active;
        return (
          <button
            key={tab}
            ref={(node) => {
              buttons.current[index] = node;
            }}
            id={projectTabId(tab)}
            type="button"
            role="tab"
            className={`tab ${selected ? "tab-active" : ""}`}
            aria-selected={selected}
            aria-controls={selected ? PROJECT_PANEL_ID : undefined}
            aria-keyshortcuts={`Meta+${index + 1}`}
            tabIndex={selected ? 0 : -1}
            title={`${PROJECT_TAB_LABEL[tab]} (⌘${index + 1})`}
            onClick={() => {
              onSelect(tab);
            }}
            onKeyDown={(event) => {
              onKeyDown(event, index);
            }}
          >
            {PROJECT_TAB_LABEL[tab]}
          </button>
        );
      })}
    </div>
  );
}
