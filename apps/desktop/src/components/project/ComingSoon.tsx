import { CalendarClock, LayoutDashboard, Package } from "lucide-react";
import type { LucideIcon } from "lucide-react";
import type { ReactElement } from "react";
import type { ProjectTab } from "../../app/selection";
import { PROJECT_TAB_LABEL } from "./ProjectTabs";

/** The tabs that are placeholders until their views land. */
export type UpcomingTab = Extract<ProjectTab, "dashboard" | "releases" | "meetings">;

const UPCOMING: Readonly<
  Record<UpcomingTab, { readonly icon: LucideIcon; readonly text: string }>
> = {
  dashboard: {
    icon: LayoutDashboard,
    text: "What needs you, the board at a glance, releases, the team and how work is flowing, in one place.",
  },
  releases: {
    icon: Package,
    text: "Release packages for you to test and approve, and where each one is deployed.",
  },
  meetings: {
    icon: CalendarClock,
    text: "Stand-ups, demos and retros the bots hold, with their notes and action items.",
  },
};

export function isUpcomingTab(tab: ProjectTab): tab is UpcomingTab {
  return tab in UPCOMING;
}

/** A project tab whose view has not shipped yet: what it will hold. */
export default function ComingSoon({ tab }: { readonly tab: UpcomingTab }): ReactElement {
  const { icon: Icon, text } = UPCOMING[tab];
  return (
    <div className="empty-pane">
      <div className="empty-state coming-soon">
        <Icon className="coming-soon-icon" aria-hidden="true" />
        <h1>{`${PROJECT_TAB_LABEL[tab]} is coming soon`}</h1>
        <p>{text}</p>
      </div>
    </div>
  );
}
