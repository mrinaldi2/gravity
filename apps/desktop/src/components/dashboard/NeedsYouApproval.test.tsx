import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { ProjectTab } from "../../app/selection";
import type { AddToast } from "../../app/useToasts";
import type { NeedsYou as NeedsYouRow } from "../../protocol/dashboard";
import { DASH_BOTS, dashboard } from "../../test/dashboardFixtures";
import { FakeDaemon } from "../../test/fakeDaemon";
import { project } from "../../test/fixtures";
import DashboardView from "./DashboardView";

function setup(needsYou: readonly NeedsYouRow[], count?: number) {
  const fake = new FakeDaemon();
  fake.onRequest("dashboard_get", () => ({
    type: "dashboard",
    req_id: "1",
    dashboard: dashboard({ needs_you: [...needsYou], needs_you_count: count }),
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
  return { nav };
}

function nth(rows: readonly HTMLElement[], index: number): HTMLElement {
  const row = rows[index];
  if (row === undefined) {
    throw new Error(`no row ${index}`);
  }
  return row;
}

describe("Needs you, a bot waiting on an approval", () => {
  it("lists a bot waiting on its terminal's approval with the decisions, opening the bot (H-172)", async () => {
    const user = userEvent.setup();
    const approval: NeedsYouRow = {
      kind: "bot_waiting",
      id: "bot_waiting:mac:arch",
      title: "Architect needs approval: Bash: curl -H 'Authorization: Bearer ***' https://x",
      bot: { daemon_id: "mac", bot_id: "arch", name: "Architect" },
    };
    const { nav } = setup([...dashboard().needs_you, approval]);
    const needs = await screen.findByRole("region", { name: "Needs you · 5" });
    const rows = within(nth(within(needs).getAllByRole("list"), 0)).getAllByRole("listitem");
    expect(rows.map((r) => r.textContent)).toEqual([
      expect.stringContaining("0.16.0 is ready for you to test"),
      expect.stringContaining("Push notifications"),
      expect.stringContaining(
        "Architect needs approval: Bash: curl -H 'Authorization: Bearer ***'",
      ),
      expect.stringContaining("Confirm 2 rulings"),
      expect.stringContaining("P0 · H-021"),
    ]);
    await user.click(within(nth(rows, 2)).getByRole("button", { name: "Open Architect" }));
    expect(nav.onOpenBot).toHaveBeenCalledWith("arch");
  });

  it("heads Needs you with the daemon's count, as the projects home's (H-161)", async () => {
    const cardless: NeedsYouRow = {
      kind: "routines_without_card",
      routines: [{ id: "r1", name: "nightly", bot_id: "dd" }],
    };
    // Four rows the home counts, and the routines row it lists without counting.
    setup([...dashboard().needs_you, cardless], 4);
    const needs = await screen.findByRole("region", { name: "Needs you · 4" });
    const rows = within(nth(within(needs).getAllByRole("list"), 0)).getAllByRole("listitem");
    expect(rows).toHaveLength(5);
  });
});
