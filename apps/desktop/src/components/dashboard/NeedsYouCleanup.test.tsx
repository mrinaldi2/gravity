import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { ProjectTab } from "../../app/selection";
import type { AddToast } from "../../app/useToasts";
import { DASH_BOTS, dashboard } from "../../test/dashboardFixtures";
import { FakeDaemon } from "../../test/fakeDaemon";
import { project } from "../../test/fixtures";
import { DISK_REPORT, HELD_ROW, LOW_ROW } from "../../test/cleanupFixtures";
import DashboardView from "./DashboardView";

function fake(): FakeDaemon {
  return new FakeDaemon()
    .onRequest("dashboard_get", () => ({
      type: "dashboard",
      req_id: "1",
      dashboard: dashboard({ needs_you: [HELD_ROW, LOW_ROW], needs_you_count: 2 }),
    }))
    .onRequest("disk_report", () => ({
      type: "disk_report",
      req_id: "2",
      disk_report: DISK_REPORT,
    }))
    .onRequest("cleanup_resolve", () => ({
      type: "cleanup_item",
      req_id: "3",
      cleanup_item: {
        job_id: "job-1",
        machine: "imac",
        state: "done",
        path: "/w/gravity-wt-desktopdev-a",
        reason: "removed by the owner",
        freed_bytes: 2_000_000_000,
        salvaged: true,
      },
    }))
    .onRequest("cleanup_now", () => ({
      type: "cleanup_done",
      req_id: "4",
      machine: "mac",
      trees: 2,
      freed_bytes: 9_100_000_000,
    }));
}

function show(daemon: FakeDaemon, addToast = vi.fn<AddToast>()) {
  render(
    <DashboardView
      client={daemon}
      project={project({ id: "p1", name: "The Hermes" })}
      bots={DASH_BOTS}
      connected
      canControl
      addToast={addToast}
      onOpenTab={vi.fn<(tab: ProjectTab) => void>()}
      onOpenBot={vi.fn<(botId: string) => void>()}
      onOpenDecision={vi.fn<(decisionId: string) => void>()}
    />,
  );
  return addToast;
}

describe("Needs you: cleanups and disk (H-275)", () => {
  it("Remove anyway sends the owner's word for that job, after a choice that starts on Cancel", async () => {
    const daemon = fake();
    const toast = show(daemon);
    const needs = await screen.findByRole("region", { name: "Needs you · 2" });
    await userEvent.click(within(needs).getByRole("button", { name: `Decide: ${HELD_ROW.title}` }));
    const dialog = await screen.findByRole("dialog", { name: "Held cleanup" });
    expect(within(dialog).getByRole("button", { name: "Cancel" })).toHaveFocus();
    await userEvent.click(within(dialog).getByRole("button", { name: "Remove anyway" }));
    expect(daemon.requests.map((r) => r.body)).toContainEqual({
      type: "cleanup_resolve",
      project_id: "p1",
      job_id: "job-1",
      action: "remove",
    });
    expect(toast).toHaveBeenCalledWith("info", "Removed", expect.stringContaining("2.0 GB"));
    expect(screen.queryByRole("dialog", { name: "Held cleanup" })).toBeNull();
  });

  it("Keep it keeps the tree", async () => {
    const daemon = fake();
    show(daemon);
    const needs = await screen.findByRole("region", { name: "Needs you · 2" });
    await userEvent.click(within(needs).getByRole("button", { name: `Decide: ${HELD_ROW.title}` }));
    await userEvent.click(await screen.findByRole("button", { name: "Keep it" }));
    expect(daemon.requests.map((r) => r.body)).toContainEqual(
      expect.objectContaining({ type: "cleanup_resolve", action: "keep" }),
    );
  });

  it("Clean up runs on the computer low on disk; the Disk widget shows each bot", async () => {
    const daemon = fake();
    const toast = show(daemon);
    const needs = await screen.findByRole("region", { name: "Needs you · 2" });
    await userEvent.click(within(needs).getByRole("button", { name: "Clean up mac" }));
    expect(daemon.requests.map((r) => r.body)).toContainEqual({
      type: "cleanup_now",
      machine: "mac",
    });
    expect(toast).toHaveBeenCalledWith("info", "Cleaned up mac", "Freed 9.1 GB.");
    const disk = await screen.findByRole("region", { name: /Disk · mac/ });
    expect(disk).toHaveTextContent("14.0 GB free of 494 GB · 9.1 GB is old build output");
    expect(disk).toHaveTextContent("Desktop Dev");
  });
});
