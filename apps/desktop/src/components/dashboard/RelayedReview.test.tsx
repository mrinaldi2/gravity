import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { ProjectTab } from "../../app/selection";
import type { AddToast } from "../../app/useToasts";
import type { Dashboard, NeedsYou, RelayedRuling } from "../../protocol/dashboard";
import { DASH_BOTS, dashboard } from "../../test/dashboardFixtures";
import { FakeDaemon } from "../../test/fakeDaemon";
import { project } from "../../test/fixtures";
import DashboardView from "./DashboardView";

const LATER: RelayedRuling = {
  id: "dec-9",
  title: "Move standups to 10:00?",
  answer: "Yes",
  bot_id: "arch",
  at: "2026-10-05T09:00:00Z",
};

/** The busy dashboard with these relayed rulings, or none. */
function withRulings(rulings: readonly RelayedRuling[]): Dashboard {
  const rows: NeedsYou[] = [];
  for (const row of dashboard().needs_you) {
    if (row.kind !== "relayed") {
      rows.push(row);
    } else if (rulings.length > 0) {
      rows.push({
        ...row,
        count: rulings.length,
        decision_ids: rulings.map((r) => r.id),
        rulings,
      });
    }
  }
  return dashboard({ needs_you: rows });
}

function setup(reads: readonly Dashboard[]) {
  const fake = new FakeDaemon();
  fake.grants = ["read", "control", "approve"];
  let read = 0;
  fake.onRequest("dashboard_get", () => {
    const data = reads[Math.min(read, reads.length - 1)] ?? dashboard();
    read += 1;
    return { type: "dashboard", req_id: "1", dashboard: data };
  });
  fake.onBoard("boardGet", () => {
    throw new Error("no board here");
  });
  const addToast = vi.fn<AddToast>();
  const onOpenDecision = vi.fn<(decisionId: string) => void>();
  render(
    <DashboardView
      client={fake}
      project={project({ name: "The Hermes" })}
      bots={DASH_BOTS}
      connected
      canControl
      addToast={addToast}
      onOpenTab={vi.fn<(tab: ProjectTab) => void>()}
      onOpenBot={vi.fn<(botId: string) => void>()}
      onOpenDecision={onOpenDecision}
    />,
  );
  return { fake, addToast, onOpenDecision };
}

async function openReview(): Promise<{ review: HTMLElement; dialog: HTMLElement }> {
  const user = userEvent.setup();
  const needs = await screen.findByRole("region", { name: "Needs you · 4" });
  const review = within(needs).getByRole("button", {
    name: "Review 2 rulings Architect recorded for you",
  });
  await user.click(review);
  return {
    review,
    dialog: screen.getByRole("dialog", { name: "Confirm 2 rulings recorded for you?" }),
  };
}

function confirms(fake: FakeDaemon): unknown[] {
  return fake.requests.map((r) => r.body).filter((b) => b.type === "confirm_relayed");
}

describe("Review N rulings…", () => {
  it("lists every ruling and what was recorded, with Cancel focused", async () => {
    const { fake } = setup([dashboard()]);
    const { dialog } = await openReview();
    expect(within(dialog).getByRole("heading")).toHaveTextContent(
      "Confirm 2 rulings recorded for you?",
    );
    expect(dialog).toHaveTextContent(
      "Architect recorded these answers on your behalf. Confirming makes each one your own " +
        "ruling; bots act on it as yours.",
    );
    const rows = within(within(dialog).getByRole("list")).getAllByRole("listitem");
    expect(rows).toHaveLength(2);
    expect(rows[0]).toHaveTextContent("Ship builds on Fridays?");
    expect(rows[0]).toHaveTextContent(/Answer: Yes, before noon only\. · by Architect · \S/);
    expect(rows[0]).not.toHaveTextContent("Never on a release week");
    expect(rows[1]).toHaveTextContent(/Keep the old app icon\?Answer: No · by Architect · \S/);
    expect(dialog).toHaveTextContent(
      "To change an answer, open it instead. Rulings you open aren't confirmed here.",
    );
    expect(
      within(dialog)
        .getAllByRole("button")
        .map((b) => b.textContent),
    ).toEqual(["Open", "Open", "Cancel", "Confirm 2 rulings"]);
    expect(within(dialog).getByRole("button", { name: "Cancel" })).toHaveFocus();
    expect(confirms(fake)).toEqual([]);
  });

  it("confirms exactly the rulings it listed, then closes", async () => {
    const user = userEvent.setup();
    const { fake, addToast } = setup([dashboard(), withRulings([])]);
    fake.onRequest("confirm_relayed", () => ({
      type: "relayed_confirmed",
      req_id: "2",
      confirmed: ["dec-7", "dec-8"],
      failed: [],
      changed: [],
    }));
    const { dialog } = await openReview();
    await user.click(within(dialog).getByRole("button", { name: "Confirm 2 rulings" }));
    expect(confirms(fake)).toEqual([
      { type: "confirm_relayed", project_id: "p1", decision_ids: ["dec-7", "dec-8"] },
    ]);
    expect(addToast).toHaveBeenCalledWith(
      "info",
      "Confirmed 2 rulings",
      "They're your own word now.",
    );
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("stays open on the reread list when the daemon says it changed", async () => {
    const user = userEvent.setup();
    const { fake } = setup([dashboard(), withRulings([LATER])]);
    fake.onRequest("confirm_relayed", () => ({
      type: "relayed_confirmed",
      req_id: "2",
      confirmed: ["dec-7"],
      failed: [],
      changed: ["dec-8"],
    }));
    const { dialog } = await openReview();
    await user.click(within(dialog).getByRole("button", { name: "Confirm 2 rulings" }));

    const reread = await screen.findByRole("dialog", {
      name: "Confirm 1 ruling recorded for you?",
    });
    expect(within(reread).getByRole("status")).toHaveTextContent(
      "The list changed while it was open. Check it again before confirming.",
    );
    expect(reread).toHaveTextContent("Move standups to 10:00?");
    expect(reread).not.toHaveTextContent("Ship builds on Fridays?");
    await user.click(within(reread).getByRole("button", { name: "Confirm 1 ruling" }));
    expect(confirms(fake)).toEqual([
      { type: "confirm_relayed", project_id: "p1", decision_ids: ["dec-7", "dec-8"] },
      { type: "confirm_relayed", project_id: "p1", decision_ids: ["dec-9"] },
    ]);
  });

  it("opens a ruling in Decisions instead of confirming it", async () => {
    const user = userEvent.setup();
    const { fake, onOpenDecision } = setup([dashboard()]);
    const { dialog } = await openReview();
    await user.click(within(dialog).getByRole("button", { name: "Open Keep the old app icon?" }));
    expect(onOpenDecision).toHaveBeenCalledWith("dec-8");
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(confirms(fake)).toEqual([]);
  });

  it("cancels on Escape and gives focus back to the Review button", async () => {
    const user = userEvent.setup();
    const { fake } = setup([dashboard()]);
    const { review } = await openReview();
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(review).toHaveFocus();
    expect(confirms(fake)).toEqual([]);
  });
});
