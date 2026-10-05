import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { ProjectTab } from "../../app/selection";
import type { AddToast } from "../../app/useToasts";
import { DASH_BOTS, dashboard } from "../../test/dashboardFixtures";
import { FakeDaemon } from "../../test/fakeDaemon";
import { project } from "../../test/fixtures";
import DashboardView from "./DashboardView";

function setup(canControl = true) {
  const fake = new FakeDaemon();
  let reads = 0;
  fake.onRequest("dashboard_get", () => {
    reads += 1;
    return { type: "dashboard", req_id: "1", dashboard: dashboard() };
  });
  const updated: unknown[] = [];
  fake.onRequest("action_update", (req) => {
    updated.push(req);
    return { type: "meeting_action", req_id: "1", action: dashboard().action_items[0] };
  });
  fake.onRequest("action_promote", (req) => {
    updated.push(req);
    return {
      type: "meeting_action",
      req_id: "1",
      action: { ...dashboard().action_items[1], item_id: "H-130" },
    };
  });
  const addToast = vi.fn<AddToast>();
  render(
    <DashboardView
      client={fake}
      project={project({ id: "p1", name: "The Hermes" })}
      bots={DASH_BOTS}
      connected
      canControl={canControl}
      addToast={addToast}
      onOpenTab={vi.fn<(tab: ProjectTab) => void>()}
      onOpenBot={vi.fn<(botId: string) => void>()}
      onOpenDecision={vi.fn<(decisionId: string) => void>()}
    />,
  );
  return { fake, updated, addToast, reads: () => reads };
}

describe("Meetings and action items", () => {
  it("shows each series' next time, the meeting collecting and the last one held", async () => {
    setup();
    const meetings = await screen.findByRole("region", { name: "Meetings" });
    const [standup, retro] = within(meetings).getAllByRole("listitem");
    expect(standup).toHaveTextContent("Daily standup");
    expect(standup).toHaveTextContent("Next:");
    expect(standup).toHaveTextContent("Last:");
    expect(standup).toHaveTextContent("2 blockers: H-021 needs a device, H-014 waits on review.");
    expect(standup).not.toHaveTextContent("Everyone else on track.");
    expect(retro).toHaveTextContent("Collecting · 4 of 6 contributed");
  });

  it("lists open actions with owner, due date, overdue flag and source meeting", async () => {
    setup();
    const actions = await screen.findByRole("region", {
      name: "Action items (3 open, 1 overdue)",
    });
    const [split, mine, alert] = within(actions).getAllByRole("listitem");
    expect(split).toHaveTextContent("Split H-014");
    expect(split).toHaveTextContent("Architect");
    expect(split).toHaveTextContent("‼ overdue");
    expect(split).toHaveTextContent("Daily standup");
    expect(mine).toHaveTextContent("You");
    expect(mine).not.toHaveTextContent("overdue");
    expect(within(alert as HTMLElement).getByRole("button", { name: "H-120" })).toBeVisible();
  });

  it("ticks, drops and promotes an action, then reads the dashboard again", async () => {
    const user = userEvent.setup();
    const { updated, addToast, reads } = setup();
    await screen.findByRole("region", { name: /^Action items/ });
    await user.click(screen.getByRole("checkbox", { name: "Mark done: Split H-014" }));
    await user.click(screen.getByRole("button", { name: "More for: Decide the release cadence" }));
    await user.click(screen.getByRole("menuitem", { name: "Promote to item" }));
    await user.click(screen.getByRole("button", { name: "More for: Add a WIP alert" }));
    // Already promoted: only done and drop.
    expect(screen.queryByRole("menuitem", { name: "Promote to item" })).toBeNull();
    await user.click(screen.getByRole("menuitem", { name: "Drop" }));
    expect(updated).toEqual([
      expect.objectContaining({ type: "action_update", action_id: "a1", status: "done" }),
      expect.objectContaining({ type: "action_promote", action_id: "a2" }),
      expect.objectContaining({ type: "action_update", action_id: "a3", status: "dropped" }),
    ]);
    expect(addToast).toHaveBeenCalledWith("info", "Promoted to H-130", expect.any(String));
    expect(reads()).toBeGreaterThanOrEqual(4);
  });

  it("reads again when the project's meetings change", async () => {
    const { fake, reads } = setup();
    await screen.findByRole("region", { name: "Meetings" });
    const before = reads();
    fake.emit("meeting_event", { type: "meeting_event", project_id: "other" });
    fake.emit("meeting_event", { type: "meeting_event", project_id: "p1" });
    await vi.waitFor(() => expect(reads()).toBe(before + 1));
  });

  it("leaves actions read-only without the control grant", async () => {
    setup(false);
    const box = await screen.findByRole("checkbox", { name: "Mark done: Split H-014" });
    expect(box).toBeDisabled();
    expect(screen.queryByRole("button", { name: /^More for/ })).toBeNull();
  });
});
