import { render, screen, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import type { PlanItem, Release } from "../../protocol/releases";
import { release } from "../../test/releaseFixtures";
import { eventLine } from "./labels";
import ReleaseProgress, { readinessLine, showsProgress } from "./ReleaseProgress";

function item(over: Partial<PlanItem> = {}): PlanItem {
  return {
    item_id: "H-137",
    title: "Planned releases",
    column_key: "doing",
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
        column_key: "verify",
        category: "verify",
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
      "1 of 3 items ready · Not built yet · Tested on 0 of 2 computers",
    );
    const rows = within(section).getAllByRole("listitem");
    expect(rows[0]).toHaveTextContent(
      /Planned releases.*Not ready · Doing · Architect · criteria 1\/3/,
    );
    expect(rows[1]).toHaveTextContent(/Guardrails.*Ready · Verify/);
    expect(rows[2]).toHaveTextContent(/Deployed via.*nobody yet.*Blocked/);
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
      "3 of 3 items ready · Built for desktop-mac · Tested on 1 of 2 computers",
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
});
