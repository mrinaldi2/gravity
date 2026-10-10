import type { Story } from "@ladle/react";
import { useState } from "react";
import type { ReactElement } from "react";
import type { ProjectTab } from "../../app/selection";
import type { AddToast } from "../../app/useToasts";
import type { NeedsYou } from "../../protocol/dashboard";
import { DASH_BOTS, dashboard } from "../../test/dashboardFixtures";
import { flowMetrics } from "../../test/flowFixtures";
import { project } from "../../test/fixtures";
import { ownerDaemon, recheckPr } from "../../test/ownerReviewFixtures";
import { readyForYouPr } from "../../test/prFixtures";
import DashboardView from "./DashboardView";

const noTab = (_tab: ProjectTab): void => {};
const noop = (): void => {};
const noToast: AddToast = () => {};

/** Your review after the bots, a Re-check, and a merge waiting for DevOps. */
const PR_ROWS: readonly NeedsYou[] = [
  {
    kind: "pr_review",
    id: "pr_review:mac:pr-42",
    title: "Review PR #42 (H-293): Waiting for you on releases",
    pr_number: 42,
  },
  {
    kind: "pr_review",
    id: "pr_review:mac:pr-45",
    title: "Review PR #45 (H-259): Search in Docs",
    pr_number: 45,
  },
  {
    kind: "pr_merge_stuck",
    id: "pr_merge_stuck:mac:pr-41",
    title: "PR #41 (H-230) is waiting for DevOps to merge it; asked 2 times",
    pr_number: 41,
  },
];

function Frame({ light }: { readonly light?: boolean }): ReactElement {
  const [client] = useState(() => {
    const data = dashboard();
    return ownerDaemon([], [readyForYouPr(), recheckPr()])
      .onRequest("dashboard_get", () => ({
        type: "dashboard",
        req_id: "1",
        dashboard: { ...data, needs_you: [...data.needs_you, ...PR_ROWS] },
      }))
      .onRequest("metrics_get", () => ({
        type: "metrics",
        req_id: "2",
        metrics: flowMetrics(),
        note: null,
      }));
  });
  return (
    <div className={`main ${light ? "theme-light" : ""}`} style={{ height: 900 }}>
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
        onOpenPr={noop}
      />
    </div>
  );
}

export const PullRequests: Story = () => <Frame />;

export const PullRequestsLight: Story = () => <Frame light />;
