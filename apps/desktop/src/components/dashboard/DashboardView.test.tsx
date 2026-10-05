import { act, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { ProjectTab } from "../../app/selection";
import type { AddToast } from "../../app/useToasts";
import type { Dashboard } from "../../protocol/dashboard";
import { DASH_BOTS, MIRRORED_BOARD, dashboard, quietDashboard } from "../../test/dashboardFixtures";
import { FakeDaemon } from "../../test/fakeDaemon";
import { project } from "../../test/fixtures";
import DashboardView from "./DashboardView";
import { REFRESH_MS } from "./useDashboard";

function setup(data: Dashboard | (() => Dashboard)) {
  const fake = new FakeDaemon();
  fake.onRequest("dashboard_get", () => ({
    type: "dashboard",
    req_id: "1",
    dashboard: typeof data === "function" ? data() : data,
  }));
  fake.onBoard("boardGet", () => {
    throw new Error("no board here");
  });
  const nav = {
    onOpenTab: vi.fn<(tab: ProjectTab) => void>(),
    onOpenBot: vi.fn<(botId: string) => void>(),
    onOpenDecision: vi.fn<(decisionId: string) => void>(),
  };
  render(
    <DashboardView
      client={fake}
      project={project({ name: "The Hermes" })}
      bots={DASH_BOTS}
      connected
      canControl
      addToast={vi.fn<AddToast>()}
      {...nav}
    />,
  );
  return { fake, nav };
}

function widget(name: RegExp): HTMLElement {
  return screen.getByRole("region", { name });
}

function nth(rows: readonly HTMLElement[], index: number): HTMLElement {
  const row = rows[index];
  if (row === undefined) {
    throw new Error(`no row ${index}`);
  }
  return row;
}

describe("DashboardView", () => {
  it("puts what needs the owner first, most urgent first, each with one action", async () => {
    const user = userEvent.setup();
    const { nav } = setup(dashboard());
    const needs = await screen.findByRole("region", { name: "Needs you · 4" });
    const rows = within(needs).getAllByRole("listitem");
    expect(rows.map((r) => r.textContent)).toEqual([
      expect.stringContaining("0.16.0 is ready for you to test · 2 items"),
      expect.stringContaining("Push notifications"),
      expect.stringContaining("P0 · H-021 Pairing crash on iOS 18.1"),
      expect.stringContaining("WIP override · Review · H-030"),
    ]);
    expect(rows[2]).toHaveTextContent("Doing · iOS Dev");
    expect(rows[3]).toHaveTextContent("WIP override: hotfix for 0.15.2 — Desktop Dev");

    await user.click(within(nth(rows, 1)).getByRole("button", { name: "Answer" }));
    expect(nav.onOpenDecision).toHaveBeenCalledWith("dec-1");
    await user.click(within(nth(rows, 2)).getByRole("button", { name: "Open board" }));
    expect(nav.onOpenTab).toHaveBeenCalledWith("board");
  });

  it("opens a release's review in a drawer, over the dashboard", async () => {
    const user = userEvent.setup();
    setup(dashboard());
    const needs = await screen.findByRole("region", { name: "Needs you · 4" });
    await user.click(nth(within(needs).getAllByRole("button", { name: "Review" }), 0));
    const drawer = screen.getByRole("complementary", { name: "Release R-2026-W41" });
    expect(within(drawer).getByRole("button", { name: "Approve 0.16.0" })).toBeInTheDocument();
    await user.click(within(drawer).getByRole("button", { name: "Close the release" }));
    expect(screen.queryByRole("complementary")).toBeNull();
  });

  it("shows the board strip with its limits, then blocked, stale and rework", async () => {
    const user = userEvent.setup();
    const { nav } = setup(dashboard());
    const board = await screen.findByRole("region", { name: /^Board/ });
    const cells = within(board)
      .getAllByRole("listitem")
      .map((c) => c.textContent);
    expect(cells).toContain("9Inbox");
    expect(cells).toContain("3Doing · 1 per bot");
    expect(cells).toContain("3/3Verify · ● Full");
    expect(cells).toContain("7Done this week");
    expect(board).toHaveTextContent("⛔ 2 blocked⏱ 3 stale↺ 1 rework this week");
    await user.click(within(board).getByRole("button", { name: "Open board ›" }));
    expect(nav.onOpenTab).toHaveBeenCalledWith("board");
  });

  it("lists releases with their state in words, and the team with workers last", async () => {
    const user = userEvent.setup();
    const { nav } = setup(dashboard());
    const releases = await screen.findByRole("region", { name: /^Releases/ });
    expect(releases).toHaveTextContent("0.16.0 ◐ Ready for you to test");
    expect(releases).toHaveTextContent("0.15.2 ✓ Live on every computer");
    expect(releases).toHaveTextContent("mac ✓ Live · win-pc ✓ Live");

    const team = widget(/^Team/);
    const names = within(team)
      .getAllByRole("listitem")
      .map((row) => row.textContent ?? "");
    expect(names.at(-1)).toContain("worker-1");
    expect(names[0]).toContain("Desktop Dev Working");
    expect(names[0]).toContain("H-002 Safe destructive actions · 1 task");
    expect(names.find((n) => n.includes("Architect"))).toContain(
      "Architect IdleNo current item · 0 tasks",
    );
    await user.click(within(team).getByRole("button", { name: /^Tester Win/ }));
    expect(nav.onOpenBot).toHaveBeenCalledWith("tw");
  });

  it("gives every widget an empty state", async () => {
    setup(quietDashboard());
    expect(await screen.findByText("Nothing needs you in The Hermes.")).toBeInTheDocument();
    expect(screen.getByText("No board yet. Start it from the Board tab.")).toBeInTheDocument();
    expect(
      screen.getByText("No release yet. DevOps packages the items in Verify."),
    ).toBeInTheDocument();
    expect(screen.getByText("No bots in this project yet.")).toBeInTheDocument();
    expect(screen.getByText("No meetings yet.")).toBeInTheDocument();
    expect(screen.getByText("No open action items.")).toBeInTheDocument();
  });

  it("says when the board lives on another computer", async () => {
    setup(
      dashboard({
        home: "mac",
        board: MIRRORED_BOARD,
      }),
    );
    const board = await screen.findByRole("region", { name: /^Board/ });
    expect(board).toHaveTextContent(
      "The board lives on mac; this is its last copy here, read-only.",
    );
    expect(board).not.toHaveTextContent("Done this week");
  });

  it("reads itself again on a timer", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    try {
      let reads = 0;
      setup(() => {
        reads += 1;
        return reads === 1 ? quietDashboard() : dashboard();
      });
      await screen.findByText("Nothing needs you in The Hermes.");
      await act(async () => vi.advanceTimersByTimeAsync(REFRESH_MS));
      expect(await screen.findByRole("region", { name: "Needs you · 4" })).toBeInTheDocument();
    } finally {
      vi.useRealTimers();
    }
  });
});
