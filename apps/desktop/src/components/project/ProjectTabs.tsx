import { useRef, useState } from "react";
import type { KeyboardEvent, ReactElement } from "react";
import { MORE_TABS, PRIMARY_TABS } from "../../app/selection";
import type { ProjectTab } from "../../app/selection";

const PROJECT_TAB_LABEL: Readonly<Record<ProjectTab, string>> = {
  overview: "Overview",
  board: "Board",
  team: "Team",
  releases: "Releases",
  meetings: "Meetings",
  conversations: "Conversations",
  settings: "Settings",
};

const MORE_ID = "project-tab-more";

/** The id of the control that labels a tab's panel: its tab, or More for the tabs under it. */
export function projectTabId(tab: ProjectTab): string {
  return MORE_TABS.some((more) => more === tab) ? MORE_ID : `project-tab-${tab}`;
}

export const PROJECT_PANEL_ID = "project-panel";

interface ProjectTabsProps {
  readonly active: ProjectTab;
  readonly onSelect: (tab: ProjectTab) => void;
}

/** Where an arrow, Home or End key moves focus from tab `index`. */
function targetIndex(key: string, index: number): number | null {
  const last = PRIMARY_TABS.length - 1;
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

/** More ▾: the project's secondary views (UX-024 §2), as a small menu. */
function MoreMenu({ active, onSelect }: ProjectTabsProps): ReactElement {
  const [open, setOpen] = useState(false);
  const current = MORE_TABS.find((tab) => tab === active);
  return (
    <div className="project-more">
      <button
        id={MORE_ID}
        type="button"
        className={`tab ${current === undefined ? "" : "tab-active"}`}
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={() => {
          setOpen((was) => !was);
        }}
      >
        {current === undefined ? "More" : PROJECT_TAB_LABEL[current]} ▾
      </button>
      {open ? (
        <div className="project-more-menu" role="menu" aria-label="More">
          {MORE_TABS.map((tab) => (
            <button
              key={tab}
              type="button"
              role="menuitem"
              className="project-more-item"
              onClick={() => {
                setOpen(false);
                onSelect(tab);
              }}
            >
              {PROJECT_TAB_LABEL[tab]}
            </button>
          ))}
        </div>
      ) : null}
    </div>
  );
}

/**
 * The project window's tab bar: a WAI-ARIA tablist of the five main views
 * with one tab stop, where the arrow keys, Home and End move between them and
 * select as they go; then More for the rest.
 */
export default function ProjectTabs({ active, onSelect }: ProjectTabsProps): ReactElement {
  const buttons = useRef<(HTMLButtonElement | null)[]>([]);
  const primary = PRIMARY_TABS.some((tab) => tab === active);

  const onKeyDown = (event: KeyboardEvent<HTMLButtonElement>, index: number): void => {
    const next = targetIndex(event.key, index);
    const tab = next === null ? undefined : PRIMARY_TABS[next];
    if (next === null || tab === undefined) {
      return;
    }
    event.preventDefault();
    onSelect(tab);
    buttons.current[next]?.focus();
  };

  return (
    <div className="tabs project-tabs">
      <div className="project-tablist" role="tablist" aria-label="Project">
        {PRIMARY_TABS.map((tab, index) => {
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
              tabIndex={selected || (!primary && index === 0) ? 0 : -1}
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
      <MoreMenu active={active} onSelect={onSelect} />
    </div>
  );
}
