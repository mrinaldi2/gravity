import { act, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { ProjectTab } from "../../app/selection";
import type { AddToast } from "../../app/useToasts";
import type { Dashboard, NeedsYou as NeedsYouRow } from "../../protocol/dashboard";
import type { Grant } from "../../protocol/entities";
import {
  DASH_BOTS,
  MIRRORED_BOARD,
  dashboard,
  offHome,
  quietDashboard,
} from "../../test/dashboardFixtures";
import { itemDetail } from "../../test/drawerFixtures";
import { FakeDaemon } from "../../test/fakeDaemon";
import { project } from "../../test/fixtures";
import DashboardView from "./DashboardView";
import { REFRESH_MS } from "./useDashboard";

function setup(data: Dashboard | (() => Dashboard), grants?: readonly Grant[]) {
  const fake = new FakeDaemon();
  if (grants) {
    fake.grants = grants;
  }
  fake.onRequest("dashboard_get", () => ({
    type: "dashboard",
    req_id: "1",
    dashboard: typeof data === "function" ? data() : data,
  }));
  fake.onBoard("boardGet", () => {
    throw new Error("no board here");
  });
  fake.onBoard("itemGet", () => {
    const detail = itemDetail();
    if (detail.item) {
      detail.item.id = "H-021";
      detail.item.title = "Pairing crash on iOS 18.1";
    }
    return { case: "item", value: detail };
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
    const rows = within(nth(within(needs).getAllByRole("list"), 0)).getAllByRole("listitem");
    expect(rows.map((r) => r.textContent)).toEqual([
      expect.stringContaining("0.16.0 is ready for you to test · 2 items"),
      expect.stringContaining("Push notifications"),
      expect.stringContaining("Confirm 2 rulings Architect recorded for you"),
      expect.stringContaining("P0 · H-021 Pairing crash on iOS 18.1"),
    ]);
    expect(rows[3]).toHaveTextContent("Doing · iOS Dev");
    expect(rows[1]).toHaveTextContent(/Raised by Architect · Answer by /);

    await user.click(within(nth(rows, 1)).getByRole("button", { name: /^Answer/ }));
    expect(nav.onOpenDecision).toHaveBeenCalledWith("dec-1");
    // An item opens its drawer over the dashboard (U4).
    const openItem = within(nth(rows, 3)).getByRole("button", { name: "Open item H-021" });
    await user.click(openItem);
    const drawer = screen.getByRole("complementary", { name: "Item H-021" });
    expect(await within(drawer).findByRole("heading", { name: /Pairing crash/ })).toHaveFocus();
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("complementary", { name: "Item H-021" })).toBeNull();
    // Focus goes back to the button that opened it (UX-012).
    expect(openItem).toHaveFocus();
  });

  it("folds the lead's WIP overrides under Needs you, out of its count", async () => {
    setup(dashboard());
    const needs = await screen.findByRole("region", { name: "Needs you · 4" });
    const folded = within(needs).getByText("1 WIP override this week");
    expect(folded.closest("details")).not.toHaveAttribute("open");
    expect(folded.closest("details")).toHaveTextContent(
      "Review · H-030 Hotfix the installer — WIP override: hotfix for 0.15.2 — Desktop Dev",
    );
  });

  it("leaves confirming rulings to the owner", async () => {
    setup(dashboard());
    const needs = await screen.findByRole("region", { name: "Needs you · 4" });
    const review = within(needs).getByRole("button", {
      name: "Review 2 rulings Architect recorded for you",
    });
    expect(review).toHaveTextContent("Review 2 rulings…");
    expect(review).toBeDisabled();
    expect(review).toHaveAttribute("title", "Only the owner can confirm rulings");
  });

  it("names every bot that recorded rulings on the relayed row", async () => {
    const by = [
      { bot_id: "dd", count: 2 },
      { bot_id: "arch", count: 1 },
    ];
    const rows: NeedsYouRow[] = [];
    for (const r of dashboard().needs_you) {
      rows.push(r.kind === "relayed" ? { ...r, count: 3, by } : r);
    }
    setup(dashboard({ needs_you: rows }));
    const needs = await screen.findByRole("region", { name: "Needs you · 4" });
    expect(needs).toHaveTextContent("Confirm 3 rulings Desktop Dev and Architect recorded for you");
    expect(
      within(needs).getByRole("button", {
        name: "Review 3 rulings Desktop Dev and Architect recorded for you",
      }),
    ).toBeInTheDocument();
  });

  it("says first when release builds aren't being served, and why", async () => {
    const reason =
      "the served directory /u/.thehermes/projects is inside the daemon's home; set [releases] dir";
    const serving: NeedsYouRow = {
      kind: "serving_off",
      title: "Release builds aren't being served",
      reason,
    };
    setup(dashboard({ needs_you: [...dashboard().needs_you, serving] }));
    const needs = await screen.findByRole("region", { name: "Needs you · 5" });
    const first = within(needs).getAllByRole("listitem")[0];
    expect(first).toHaveTextContent(`Release builds aren't being served${reason}`);
    expect(within(first).queryByRole("button")).toBeNull();
  });

  it("shows the home's rows off-home, each saying what to do there", async () => {
    setup(offHome());
    const needs = await screen.findByRole("region", { name: "Needs you · 4" });
    const there = ["Review on mac", "Answer on mac", "Confirm on mac"].map((text) =>
      within(needs).getByText(text),
    );
    for (const text of there) {
      expect(text.tagName).toBe("SPAN");
      expect(text.closest("li")).toHaveAttribute("aria-describedby", text.id);
    }
    expect(within(needs).getAllByRole("listitem")[2]).toHaveAccessibleDescription("Confirm on mac");
    expect(within(needs).queryByText("On mac")).toBeNull();
    expect(
      within(needs)
        .getAllByRole("button")
        .map((b) => b.textContent),
      // The P0 row's, then the folded override's.
    ).toEqual(["Open item", "Open item"]);
  });

  it("with the home away, says this may not be everything, and never 'Nothing'", async () => {
    const note = "Can't reach mac right now, so this may not be everything that needs you.";
    setup(dashboard({ home: "mac", needs_you: [], needs_you_note: note }));
    const needs = await screen.findByRole("region", { name: "Needs you" });
    expect(within(needs).getByText(note).tagName).toBe("STRONG");
    expect(needs).not.toHaveTextContent("Nothing needs you");
  });

  it("with the home away, still lists this computer's rows under the note", async () => {
    const note = "Can't reach mac right now, so this may not be everything that needs you.";
    const local = dashboard().needs_you.filter((r) => r.kind === "decision");
    setup(dashboard({ home: "mac", needs_you: local, needs_you_note: note }));
    const needs = await screen.findByRole("region", { name: "Needs you · 1" });
    expect(needs).toHaveTextContent(note);
    expect(within(needs).getByRole("button", { name: /^Answer/ })).toBeInTheDocument();
  });

  it("names each row's action after what it acts on, starting with its visible words", async () => {
    setup(dashboard());
    const needs = await screen.findByRole("region", { name: "Needs you · 4" });
    const buttons = within(needs).getAllByRole("button");
    expect(buttons.map((b) => b.getAttribute("aria-label"))).toEqual([
      "Review 0.16.0",
      "Answer: Push notifications: pay for an APNs relay?",
      "Review 2 rulings Architect recorded for you",
      "Open item H-021",
      // The folded WIP override's (UX-016 follow-up 2).
      "Open item H-030",
    ]);
    // "Review 2 rulings…" is said without its ellipsis.
    for (const button of buttons) {
      const words = (button.textContent ?? "").replace(/…$/, "");
      expect(button.getAttribute("aria-label")?.startsWith(words)).toBe(true);
    }
  });

  it("opens a release's review in a drawer, over the dashboard", async () => {
    const user = userEvent.setup();
    setup(dashboard());
    const needs = await screen.findByRole("region", { name: "Needs you · 4" });
    await user.click(within(needs).getByRole("button", { name: "Review 0.16.0" }));
    const drawer = screen.getByRole("complementary", { name: "Release R-2026-W41" });
    expect(within(drawer).getByRole("button", { name: "Approve 0.16.0" })).toBeInTheDocument();
    await user.click(within(drawer).getByRole("button", { name: "Close the release" }));
    expect(screen.queryByRole("complementary")).toBeNull();
  });

  it("moves focus into the drawer, and back to its Review button when Escape closes it", async () => {
    const user = userEvent.setup();
    setup(dashboard());
    const needs = await screen.findByRole("region", { name: "Needs you · 4" });
    const review = within(needs).getByRole("button", { name: "Review 0.16.0" });
    review.focus();
    await user.keyboard("{Enter}");
    const drawer = screen.getByRole("complementary", { name: "Release R-2026-W41" });
    expect(within(drawer).getByRole("button", { name: "Close the release" })).toHaveFocus();

    await user.keyboard("{Escape}");
    expect(screen.queryByRole("complementary")).toBeNull();
    expect(review).toHaveFocus();
  });

  it("returns focus to the Review button when the close button shuts the drawer", async () => {
    const user = userEvent.setup();
    setup(dashboard());
    const needs = await screen.findByRole("region", { name: "Needs you · 4" });
    const review = within(needs).getByRole("button", { name: "Review 0.16.0" });
    await user.click(review);
    await user.click(screen.getByRole("button", { name: "Close the release" }));
    expect(review).toHaveFocus();
  });

  it("leaves Escape to a dialog open inside the drawer", async () => {
    const user = userEvent.setup();
    setup(dashboard());
    const needs = await screen.findByRole("region", { name: "Needs you · 4" });
    await user.click(within(needs).getByRole("button", { name: "Review 0.16.0" }));
    const drawer = screen.getByRole("complementary", { name: "Release R-2026-W41" });
    await user.click(within(drawer).getByRole("button", { name: "Approve 0.16.0" }));
    expect(within(drawer).getByRole("dialog")).toBeInTheDocument();

    await user.keyboard("{Escape}");
    expect(within(drawer).queryByRole("dialog")).toBeNull();
    expect(screen.getByRole("complementary", { name: "Release R-2026-W41" })).toBeInTheDocument();
    // The next Escape is the drawer's (UX-012).
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("complementary")).toBeNull();
  });

  it("opens a folded WIP override's item in its drawer, and gives focus back on Esc", async () => {
    const user = userEvent.setup();
    setup(dashboard());
    const needs = await screen.findByRole("region", { name: "Needs you · 4" });
    await user.click(within(needs).getByText("1 WIP override this week"));
    const openItem = within(needs).getByRole("button", { name: "Open item H-030" });
    expect(openItem).toHaveTextContent("Open item");
    await user.click(openItem);
    expect(screen.getByRole("complementary", { name: "Item H-030" })).toBeInTheDocument();
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("complementary")).toBeNull();
    expect(openItem).toHaveFocus();
  });

  it("keeps one drawer open at a time, focus going back to the latest opener", async () => {
    const user = userEvent.setup();
    setup(dashboard());
    const needs = await screen.findByRole("region", { name: "Needs you · 4" });
    await user.click(within(needs).getByRole("button", { name: "Review 0.16.0" }));
    const openItem = within(needs).getByRole("button", { name: "Open item H-021" });
    await user.click(openItem);
    expect(screen.queryByRole("complementary", { name: "Release R-2026-W41" })).toBeNull();
    const drawer = screen.getByRole("complementary", { name: "Item H-021" });
    await within(drawer).findByRole("heading", { name: /Pairing crash/ });
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("complementary")).toBeNull();
    expect(openItem).toHaveFocus();
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
    // No "· 0": the sentence already says nothing needs you.
    expect(screen.getByRole("heading", { name: "Needs you" })).toBeInTheDocument();
    expect(screen.getByText("No board yet. Start it from the Board tab.")).toBeInTheDocument();
    expect(
      screen.getByText("No release yet. DevOps packages items once they pass Verify."),
    ).toBeInTheDocument();
    expect(screen.getByText("No bots in this project yet.")).toBeInTheDocument();
    expect(screen.getByText("No meetings set up. The lead sets up a series.")).toBeInTheDocument();
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
