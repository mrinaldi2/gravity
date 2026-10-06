import type { Story } from "@ladle/react";
import type { ReactElement } from "react";
import type { ProjectTab } from "../../app/selection";
import type { AddToast } from "../../app/useToasts";
import type { Dashboard } from "../../protocol/dashboard";
import type { FlowMetrics } from "../../protocol/metrics";
import { flowMetrics } from "../../test/flowFixtures";
import {
  DASH_BOTS,
  MIRRORED_BOARD,
  dashboard,
  offHome,
  quietDashboard,
} from "../../test/dashboardFixtures";
import { FakeDaemon } from "../../test/fakeDaemon";
import { project } from "../../test/fixtures";
import ProjectWindow from "../project/ProjectWindow";
import DashboardView from "./DashboardView";
import RelayedDialog from "./RelayedDialog";

const noTab = (_tab: ProjectTab): void => {};
const noop = (): void => {};
const noToast: AddToast = () => {};
const botName = (id: string): string => DASH_BOTS.find((b) => b.id === id)?.name ?? "a bot";

const WEEK = { metrics: flowMetrics(), note: null };

function Window({
  data,
  flow = WEEK,
}: {
  readonly data: Dashboard;
  readonly flow?: { readonly metrics: FlowMetrics | null; readonly note: string | null };
}): ReactElement {
  const hermes = project({ id: "p1", name: "The Hermes" });
  const client = new FakeDaemon()
    .onRequest("dashboard_get", () => ({
      type: "dashboard",
      req_id: "1",
      dashboard: data,
    }))
    .onRequest("metrics_get", () => ({ type: "metrics", req_id: "2", ...flow }));
  // The header counts the same bots the Team widget lists (UX-010 QA).
  const bots = data.team.map((row) => row.bot);
  return (
    <div className="main" style={{ height: 900 }}>
      <ProjectWindow project={hermes} botCount={bots.length} tab="overview" onSelectTab={noTab}>
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

/** A working week: a release to test, a decision, relayed rulings, a P0, and
 *  an override folded below. */
export const Busy: Story = () => <Window data={dashboard()} />;
/** On imac or win-pc: the home's rows to act on there. */
export const OffHome: Story = () => <Window data={offHome({ releases: [] })} />;
/** Off-home with the home away: where to look. */
export const HomeAway: Story = () => (
  <Window
    data={dashboard({
      home: "mac",
      board: MIRRORED_BOARD,
      releases: [],
      needs_you: [],
      wip_overrides: [],
      meetings: [],
      action_items: [],
      needs_you_note: "Can't reach mac right now, so this may not be everything that needs you.",
    })}
  />
);
/** "Review 2 rulings…" open: what one confirm would make the owner's own. */
export const RelayedReview: Story = () => {
  const data = dashboard();
  const relayed = data.needs_you.find((r) => r.kind === "relayed");
  return (
    <>
      <Window data={data} />
      <RelayedDialog
        rulings={relayed?.kind === "relayed" ? relayed.rulings : []}
        botName={botName}
        confirming={false}
        onConfirm={() => Promise.resolve("done")}
        onOpen={noop}
        onClose={noop}
      />
    </>
  );
};
/** A new project: every widget's empty state. */
export const Quiet: Story = () => (
  <Window data={quietDashboard()} flow={{ metrics: null, note: null }} />
);
/** On imac or win-pc: the board as mirrored from its home. */
export const Mirrored: Story = () => (
  <Window
    data={dashboard({
      home: "mac",
      needs_you: [],
      releases: [],
      meetings: [],
      action_items: [],
      board: MIRRORED_BOARD,
    })}
  />
);
