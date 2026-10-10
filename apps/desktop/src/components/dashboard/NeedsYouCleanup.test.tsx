import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { ProjectTab } from "../../app/selection";
import type { AddToast } from "../../app/useToasts";
import type { AttentionRowJson } from "../../protocol/dashboard";
import { DISK_REPORT, FAILED_ROW, HELD_ROW, LOW_ROW } from "../../test/cleanupFixtures";
import { DASH_BOTS, dashboard } from "../../test/dashboardFixtures";
import { FakeDaemon } from "../../test/fakeDaemon";
import { project } from "../../test/fixtures";
import DashboardView from "./DashboardView";

const DECIDE_HELD = "Decide: Desktop Dev's worktree on imac";
const DECIDE_FAILED = "Decide: iOS Dev's worktree on win-pc";

/** A service whose Needs you drops the held row once it is answered. */
function fake(refuse: string | null = null): FakeDaemon {
  let rows: AttentionRowJson[] = [HELD_ROW, FAILED_ROW, LOW_ROW];
  const daemon = new FakeDaemon()
    .onRequest("dashboard_get", () => ({
      type: "dashboard",
      req_id: "1",
      dashboard: dashboard({ needs_you: rows, needs_you_count: rows.length }),
    }))
    .onRequest("disk_report", () => ({
      type: "disk_report",
      req_id: "2",
      disk_report: DISK_REPORT,
    }))
    .onRequest("cleanup_resolve", () => {
      if (refuse !== null) {
        throw new Error(refuse);
      }
      rows = rows.filter((r) => r !== HELD_ROW);
      return {
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
      };
    })
    .onRequest("cleanup_now", () => ({
      type: "cleanup_done",
      req_id: "4",
      machine: "mac",
      trees: 2,
      freed_bytes: 9_100_000_000,
    }));
  return daemon;
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

async function needsYou() {
  return screen.findByRole("region", { name: /^Needs you/ });
}

describe("Needs you: cleanups and disk (H-275, UX-055)", () => {
  it("Remove anyway names the tree, starts on Cancel, and hands focus to the next row", async () => {
    const daemon = fake();
    const toast = show(daemon);
    const needs = await needsYou();
    await userEvent.click(within(needs).getByRole("button", { name: DECIDE_HELD }));
    const dialog = await screen.findByRole("dialog", {
      name: "Remove Desktop Dev's worktree on imac?",
    });
    expect(dialog).toHaveTextContent(
      "1 uncommitted file. Its changes are saved in the salvage folder.",
    );
    expect(within(dialog).getByRole("button", { name: "Cancel" })).toHaveFocus();
    await userEvent.click(within(dialog).getByRole("button", { name: "Remove anyway" }));
    expect(daemon.requests.map((r) => r.body)).toContainEqual({
      type: "cleanup_resolve",
      project_id: "p1",
      job_id: "job-1",
      action: "remove",
    });
    expect(toast).toHaveBeenCalledWith("info", "Removed", expect.stringContaining("2.0 GB"));
    await waitFor(() =>
      expect(within(needs).getByRole("button", { name: DECIDE_FAILED })).toHaveFocus(),
    );
  });

  it("Cancel and Escape give focus back to Decide…", async () => {
    show(fake());
    const needs = await needsYou();
    const decide = within(needs).getByRole("button", { name: DECIDE_HELD });
    await userEvent.click(decide);
    await userEvent.click(await screen.findByRole("button", { name: "Cancel" }));
    expect(decide).toHaveFocus();
    await userEvent.click(decide);
    await screen.findByRole("dialog");
    await userEvent.keyboard("{Escape}");
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(decide).toHaveFocus();
  });

  it("a failed tree whose changes couldn't be saved offers Keep it only", async () => {
    const daemon = fake();
    show(daemon);
    const needs = await needsYou();
    expect(needs).toHaveTextContent(
      "Couldn't remove iOS Dev's worktree on win-pc: the folder is in use",
    );
    await userEvent.click(within(needs).getByRole("button", { name: DECIDE_FAILED }));
    const dialog = await screen.findByRole("dialog", {
      name: "Keep iOS Dev's worktree on win-pc?",
    });
    expect(dialog).toHaveTextContent(
      "Remove anyway needs its changes saved first, and they couldn't be: the folder is in use.",
    );
    expect(within(dialog).queryByRole("button", { name: "Remove anyway" })).toBeNull();
    await userEvent.click(within(dialog).getByRole("button", { name: "Keep it" }));
    expect(daemon.requests.map((r) => r.body)).toContainEqual(
      expect.objectContaining({ type: "cleanup_resolve", job_id: "job-2", action: "keep" }),
    );
  });

  it("a refusal shows in the choice, which stops offering both answers", async () => {
    const toast = show(fake("Do it on mac or your phone."));
    const needs = await needsYou();
    await userEvent.click(within(needs).getByRole("button", { name: DECIDE_HELD }));
    const dialog = await screen.findByRole("dialog");
    await userEvent.click(within(dialog).getByRole("button", { name: "Remove anyway" }));
    expect(await within(dialog).findByRole("alert")).toHaveTextContent(
      "Do it on mac or your phone.",
    );
    expect(within(dialog).getByRole("button", { name: "Remove anyway" })).toBeDisabled();
    expect(within(dialog).getByRole("button", { name: "Keep it" })).toBeDisabled();
    expect(toast).toHaveBeenCalledWith(
      "error",
      "Couldn't remove the worktree",
      expect.stringContaining("Do it on mac"),
    );
  });

  it("Clean up runs on the computer low on disk; the Disk widget says so and shows each bot", async () => {
    const daemon = fake();
    const toast = show(daemon);
    const needs = await needsYou();
    await userEvent.click(within(needs).getByRole("button", { name: "Clean up mac" }));
    expect(daemon.requests.map((r) => r.body)).toContainEqual({
      type: "cleanup_now",
      machine: "mac",
    });
    expect(toast).toHaveBeenCalledWith("info", "Cleaned up mac", "Freed 9.1 GB.");
    const disk = await screen.findByRole("region", { name: /^Disk · mac/ });
    expect(disk).toHaveTextContent(
      "⚠ Low on disk · 14.0 GB free of 494 GB · 9.1 GB is old build output",
    );
    expect(disk).toHaveTextContent("Checked");
    expect(within(disk).getByRole("button", { name: "Check now" })).toBeInTheDocument();
    expect(within(disk).getByRole("button", { name: "Clean up mac" })).toBeInTheDocument();
    expect(disk).toHaveTextContent("iOS Dev13.1 GB · 9.1 GB old build output · 3.1 GB worktrees");
  });
});
