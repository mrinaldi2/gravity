import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ReactElement } from "react";
import { describe, expect, it } from "vitest";
import type { PlanItem, Release } from "../../protocol/releases";
import { FakeDaemon } from "../../test/fakeDaemon";
import { release } from "../../test/releaseFixtures";
import { eventLine } from "./labels";
import ReleaseProgress, { leftSentence, readinessLine, showsProgress } from "./ReleaseProgress";
import ReleaseReview from "./ReleaseReview";
import { useReleaseActions } from "./useReleases";

function Review({ value }: { readonly value: Release }): ReactElement {
  const actions = useReleaseActions(
    new FakeDaemon(),
    () => undefined,
    () => undefined,
  );
  return (
    <ReleaseReview
      release={value}
      titles={new Map()}
      botName={() => undefined}
      actions={actions}
      canControl
    />
  );
}

function item(over: Partial<PlanItem> = {}): PlanItem {
  return {
    item_id: "H-137",
    title: "Planned releases",
    column_key: "doing",
    column_name: "In progress",
    category: "doing",
    assignee: "arch",
    blocked: false,
    ac_checked: 1,
    ac_total: 3,
    ready: false,
    ...over,
  };
}

function planned(over: Partial<Release> = {}): Release {
  return release({
    status: "planned",
    decision_id: null,
    builds: [],
    tests: [],
    plan: [
      item(),
      item({
        item_id: "H-125",
        title: "Guardrails",
        column_key: "approval",
        column_name: "Awaiting owner",
        category: "approval",
        ac_checked: 3,
        ready: true,
      }),
      item({ item_id: "H-121", title: "Deployed via", blocked: true, assignee: null }),
    ],
    readiness: {
      items_total: 3,
      items_ready: 1,
      builds: [],
      tests_required: ["mac", "win-pc"],
      tests_passed: [],
    },
    ...over,
  });
}

const names = (id: string): string | undefined => (id === "arch" ? "Architect" : undefined);

describe("ReleaseProgress", () => {
  it("shows how far a planned release is, item by item", () => {
    render(<ReleaseProgress release={planned()} botName={names} />);
    const section = screen.getByRole("region", { name: "Progress" });
    expect(section).toHaveTextContent(
      "Packaging starts when every item reaches Verify: 2 to go (H-137 in In progress, H-121 in In progress, ⛔ blocked).",
    );
    expect(section).toHaveTextContent(
      "1 of 3 items ready · Not built yet · Tested on 0 of 2 computers",
    );
    const rows = within(section).getAllByRole("listitem");
    expect(rows[0]).toHaveTextContent(
      /○ Still in progress.*Planned releases.*In progress · Architect · ☑ 1\/3 AC/,
    );
    expect(rows[0]).toHaveTextContent("1 of 3 acceptance criteria");
    expect(rows[1]).toHaveTextContent(/✓ Ready for the release.*Guardrails.*Awaiting owner/);
    expect(rows[2]).toHaveTextContent(/Deployed via.*Unassigned.*⛔ Blocked/);
  });

  it("says what is left in each state (UX-025 §5)", () => {
    const many = planned({
      plan: ["H-1", "H-2", "H-3", "H-4", "H-5"].map((id) =>
        item({ item_id: id, column_name: undefined, column_key: "review" }),
      ),
      readiness: {
        items_total: 5,
        items_ready: 0,
        builds: [],
        tests_required: ["mac"],
        tests_passed: [],
      },
    });
    expect(leftSentence(many)).toBe(
      "Packaging starts when every item reaches Verify: 5 to go (H-1 in Review, H-2 in Review, H-3 in Review, +2 more).",
    );
    const ready = planned({ plan: [item({ ready: true })] });
    expect(leftSentence(ready)).toBe(
      "Every item has reached Verify. DevOps can start packaging it now.",
    );
    expect(leftSentence(planned({ status: "assembling" }))).toBe(
      "DevOps is building it. Next: tests on 2 computers.",
    );
    const someBuilds = planned({
      status: "assembling",
      readiness: {
        items_total: 3,
        items_ready: 3,
        builds: ["desktop-mac", "desktop-win"],
        tests_required: ["mac"],
        tests_passed: [],
      },
    });
    expect(leftSentence(someBuilds)).toBe(
      "DevOps is building it: built for Mac and Windows so far. Next: tests on 1 computer.",
    );
    expect(leftSentence(planned({ status: "built" }))).toBe(
      "Being tested: 0 of 2 computers passed. It comes to you to test when all of them pass.",
    );
    const passed = planned({
      status: "built",
      readiness: {
        items_total: 3,
        items_ready: 3,
        builds: ["desktop-mac"],
        tests_required: ["mac"],
        tests_passed: ["mac"],
      },
    });
    expect(leftSentence(passed)).toBe(
      "Every computer passed. DevOps sends it to you to test next.",
    );
  });

  it("is shown only until the package is submitted", () => {
    expect(showsProgress(planned())).toBe(true);
    expect(showsProgress(planned({ status: "built" }))).toBe(true);
    expect(showsProgress(planned({ status: "awaiting_owner" }))).toBe(false);
    expect(showsProgress(planned({ plan: [] }))).toBe(false);
  });

  it("counts builds and passed computers once they come", () => {
    const built = planned({
      status: "built",
      readiness: {
        items_total: 3,
        items_ready: 3,
        builds: ["desktop-mac"],
        tests_required: ["mac", "win-pc"],
        tests_passed: ["mac"],
      },
    });
    expect(readinessLine(built)).toBe(
      "3 of 3 items ready · Built for Mac · Tested on 1 of 2 computers",
    );
  });

  it("puts scope changes on the record in words", () => {
    const base = {
      release_id: "r",
      release_name: "0.16.4",
      related_id: null,
      actor: "arch",
      at: "2026-10-06T09:00:00Z",
    };
    expect(
      eventLine(
        {
          ...base,
          kind: "items_changed",
          note: "slips",
          detail: { added: ["H-1"], removed: ["H-2"] },
        },
        names,
      ),
    ).toBe("Architect added H-1 and took out H-2 in 0.16.4. Why: “slips”.");
    expect(
      eventLine({ ...base, kind: "planned", note: null, detail: { items: ["H-1", "H-2"] } }, names),
    ).toBe("Architect planned 0.16.4 with H-1, H-2.");
  });

  it("says nothing of builds and tests while planned, and lists the plan's items (H-142)", async () => {
    render(
      <Review
        value={planned({
          items: ["H-137", "H-125", "H-121"].map((item_id) => ({
            item_id,
            verdict: "pending" as const,
            owner_note: null,
          })),
        })}
      />,
    );
    expect(screen.queryByText("No builds yet")).not.toBeInTheDocument();
    expect(screen.queryByRole("region", { name: "Tests" })).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole("tab", { name: "Items 3" }));
    // Titles come from the plan when the board fetch has none.
    expect(screen.getByRole("tabpanel")).toHaveTextContent("Deployed via");
  });
});
