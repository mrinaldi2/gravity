import type { Story } from "@ladle/react";
import type { ReactElement } from "react";
import type { ProjectTab } from "../../app/selection";
import type { AddToast } from "../../app/useToasts";
import type { Dashboard } from "../../protocol/dashboard";
import { MIRRORED_BOARD, dashboard, quietDashboard } from "../../test/dashboardFixtures";
import { FakeDaemon } from "../../test/fakeDaemon";
import { project } from "../../test/fixtures";
import ProjectWindow from "../project/ProjectWindow";
import DashboardView from "./DashboardView";

const noTab = (_tab: ProjectTab): void => {};
const noop = (): void => {};
const noToast: AddToast = () => {};

function Window({ data }: { readonly data: Dashboard }): ReactElement {
  const hermes = project({ id: "p1", name: "The Hermes" });
  const client = new FakeDaemon().onRequest("dashboard_get", () => ({
    type: "dashboard",
    req_id: "1",
    dashboard: data,
  }));
  // The header counts the same bots the Team widget lists (UX-010 QA).
  const bots = data.team.map((row) => row.bot);
  return (
    <div className="main" style={{ height: 900 }}>
      <ProjectWindow
        project={hermes}
        botCount={bots.length}
        tab="dashboard"
        onSelectTab={noTab}
      >
        <DashboardView
          client={client}
          project={hermes}
          bots={bots}
          connected
          canControl
          addToast={noToast}
          onOpenTab={noTab}
          onOpenBot={noop}
          onOpenDecision={noop}
        />
      </ProjectWindow>
    </div>
  );
}

/** A working week: a release to test, a decision, a P0 and an override. */
export const Busy: Story = () => <Window data={dashboard()} />;
/** A new project: every widget's empty state. */
export const Quiet: Story = () => <Window data={quietDashboard()} />;
/** On imac or win-pc: the board as mirrored from its home. */
export const Mirrored: Story = () => (
  <Window
    data={dashboard({
      home: "mac",
      needs_you: [],
      releases: [],
      board: MIRRORED_BOARD,
    })}
  />
);
