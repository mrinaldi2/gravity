import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { ProjectTab } from "../../app/selection";
import type { AddToast } from "../../app/useToasts";
import { DASH_BOTS, dashboard } from "../../test/dashboardFixtures";
import { project } from "../../test/fixtures";
import { ownerDaemon } from "../../test/ownerReviewFixtures";
import DashboardView from "./DashboardView";

describe("Needs you: pull requests in the Overview", () => {
  it("reads the PRs to say who approved, and Review… opens the PR", async () => {
    const fake = ownerDaemon();
    fake.onRequest("dashboard_get", () => ({
      type: "dashboard",
      req_id: "1",
      dashboard: dashboard({
        needs_you: [
          {
            kind: "pr_review",
            id: "pr_review:mac:pr-42",
            title: "Review PR #42 (H-293): Waiting for you on releases",
            pr_number: 42,
          },
        ],
        needs_you_count: 1,
      }),
    }));
    const onOpenPr = vi.fn<(pr: number, recheck: boolean) => void>();
    render(
      <DashboardView
        client={fake}
        project={project({ id: "p1", name: "The Hermes" })}
        bots={DASH_BOTS}
        connected
        canControl
        addToast={vi.fn<AddToast>()}
        onOpenTab={vi.fn<(tab: ProjectTab) => void>()}
        onOpenBot={vi.fn<(botId: string) => void>()}
        onOpenDecision={vi.fn<(decisionId: string) => void>()}
        onOpenPr={onOpenPr}
      />,
    );
    const needs = await screen.findByRole("region", { name: "Needs you · 1" });
    const button = await within(needs).findByRole("button", { name: "Review pull request #42" });
    const item = button.closest("li");
    expect(item).toHaveTextContent("Review pull request #42 · H-293 Waiting for you on releases");
    expect(await within(needs).findByText(/Architect ✓ UX ✓ CE ✓/)).toBeInTheDocument();
    await userEvent.click(button);
    expect(onOpenPr).toHaveBeenCalledWith(42, false);
  });
});
