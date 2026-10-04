import type { ReactElement } from "react";

interface ProjectRowProps {
  readonly selected: boolean;
  readonly onSelect: () => void;
}

function WindowIcon(): ReactElement {
  return (
    <svg viewBox="0 0 16 16" className="icon" aria-hidden="true">
      <rect x="2.5" y="2.5" width="11" height="11" rx="2" />
      <path d="M2.5 6h11M6.5 6v7.5" />
    </svg>
  );
}

/** Opens the project window, for people who do not click the project's name. */
export default function ProjectRow({ selected, onSelect }: ProjectRowProps): ReactElement {
  return (
    <button
      type="button"
      className={`row project-row ${selected ? "row-selected" : ""}`}
      title="Dashboard, board, releases, meetings, conversations and settings"
      aria-current={selected ? "page" : undefined}
      onClick={onSelect}
    >
      <WindowIcon />
      <span className="project-row-name">Project</span>
    </button>
  );
}
