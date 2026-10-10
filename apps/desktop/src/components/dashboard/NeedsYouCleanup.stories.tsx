import type { Story } from "@ladle/react";
import { useState } from "react";
import type { ReactElement } from "react";
import type { ProjectTab } from "../../app/selection";
import type { AddToast } from "../../app/useToasts";
import { DISK_REPORT, FAILED_ROW, HELD_ROW, LOW_ROW } from "../../test/cleanupFixtures";
import { DASH_BOTS, dashboard } from "../../test/dashboardFixtures";
import { FakeDaemon } from "../../test/fakeDaemon";
import { project } from "../../test/fixtures";
import { flowMetrics } from "../../test/flowFixtures";
import DashboardView from "./DashboardView";

const noTab = (_tab: ProjectTab): void => {};
const noop = (): void => {};
const noToast: AddToast = () => {};

/** A cleanup held 3 days, a computer low on disk, and the Disk widget (H-275). */
function Frame({ light }: { readonly light?: boolean }): ReactElement {
  const [client] = useState(() => {
    const data = dashboard();
    return new FakeDaemon()
      .onRequest("dashboard_get", () => ({
        type: "dashboard",
        req_id: "1",
        dashboard: { ...data, needs_you: [LOW_ROW, ...data.needs_you, HELD_ROW, FAILED_ROW] },
      }))
      .onRequest("metrics_get", () => ({
        type: "metrics",
        req_id: "2",
        metrics: flowMetrics(),
        note: null,
      }))
      .onRequest("disk_report", () => ({
        type: "disk_report",
        req_id: "3",
        disk_report: DISK_REPORT,
      }));
  });
  return (
    <div className={`main ${light ? "theme-light" : ""}`} style={{ height: 1100 }}>
      <DashboardView
        client={client}
        project={project({ id: "p1", name: "The Hermes" })}
        bots={DASH_BOTS}
        connected
        canControl
        addToast={noToast}
        onOpenTab={noTab}
        onOpenBot={noop}
        onOpenDecision={noop}
      />
    </div>
  );
}

export const Cleanup: Story = () => <Frame />;

export const CleanupLight: Story = () => <Frame light />;
