import { create } from "@bufbuild/protobuf";
import { describe, expect, it } from "vitest";
import {
  CheckResult,
  CheckRunSchema,
  CleanupKind,
  CleanupSchema,
  CleanupState,
  PrState,
} from "../../protocol/gen/hermes/pr/v1/pr_pb";
import {
  HEAD,
  mergedPr,
  OLDER,
  PR_NOW,
  prList,
  readyForYouPr,
  waitingPr,
} from "../../test/prFixtures";
import { checksSummary, cleanupLine, splitChecks } from "./checkText";
import {
  age,
  branchMarker,
  joinNames,
  reviewChip,
  reviewOf,
  reviewRoles,
  reviewState,
  stateLine,
  waitingFor,
  waitsForYou,
} from "./prText";

const [ready, behind, conflict] = prList();

function run(result: CheckResult) {
  return create(CheckRunSchema, { name: "x", result });
}

describe("reviewer states (UX-051 decision 3)", () => {
  it("binds each verdict to its commit, with a glyph and a word", () => {
    const pr = waitingPr();
    expect(reviewState(pr, "architect", reviewOf(pr, "architect"))).toMatchObject({
      glyph: "✓",
      text: "Approved {sha}",
      sha: HEAD,
    });
    expect(reviewState(pr, "ce", reviewOf(pr, "ce"))).toMatchObject({
      glyph: "⟳",
      text: "Approved {sha}, an older commit",
      sha: OLDER,
    });
    expect(reviewState(conflict, "architect", reviewOf(conflict, "architect")).text).toBe(
      "Asked for changes",
    );
    expect(reviewState(pr, "qa", undefined)).toMatchObject({ glyph: "○", text: "Waiting" });
  });

  it("asks for your review only once the reviewers are in", () => {
    expect(waitsForYou(waitingPr())).toBe(false);
    expect(reviewState(waitingPr(), "owner", undefined).text).toBe("Waiting");
    expect(waitsForYou(readyForYouPr())).toBe(true);
    expect(reviewState(readyForYouPr(), "owner", undefined).text).toBe("Waiting for your review");
  });

  it("lists you first, then the required roles", () => {
    expect(reviewRoles(waitingPr())).toEqual(["owner", "architect", "ux", "ce"]);
  });

  it("words the list's chips", () => {
    const pr = waitingPr();
    expect(reviewRoles(pr).map((role) => reviewChip(pr, role).text)).toEqual([
      "○ You after reviewers",
      "✓ Architect",
      "✓ UX",
      "⟳ CE approved an older commit",
    ]);
    expect(reviewChip(ready ?? pr, "owner").text).toBe("▲ You");
    expect(reviewChip(conflict ?? pr, "architect").text).toBe("✕ Architect asked for changes");
  });
});

describe("what a PR waits for (decisions 4 and 11)", () => {
  it("names who is missing, you first", () => {
    expect(waitingFor(waitingPr())).toBe("you, and CE");
    expect(waitingFor(readyForYouPr())).toBe("you");
    expect(behind && waitingFor(behind)).toBe("you, and checks");
    expect(joinNames(["you", "CE", "checks"])).toBe("you, CE, and checks");
    expect(joinNames([])).toBe("");
  });

  it("marks conflicts before behind main", () => {
    expect(behind && branchMarker(behind)?.text).toBe("⇣ Needs update with main");
    expect(conflict && branchMarker(conflict)?.text).toBe(
      "⚠ Has conflicts: waiting for Backend Dev to resolve",
    );
    expect(branchMarker(mergedPr())).toBeNull();
  });

  it("says when it merged, in local time", () => {
    const merged = mergedPr();
    const at = new Date(PR_NOW - 168 * 60_000);
    const hhmm = `${String(at.getHours()).padStart(2, "0")}:${String(at.getMinutes()).padStart(2, "0")}`;
    expect(stateLine(merged, PR_NOW)).toBe(`✓ Merged into main · ${hhmm}`);
    expect(stateLine(waitingPr(), PR_NOW)).toBeNull();
    expect(stateLine({ ...merged, state: PrState.MERGING }, PR_NOW)).toBe("◌ Merging into main…");
  });

  it("gives ages in minutes, hours and days", () => {
    const pr = waitingPr();
    expect(age(pr.openedAt, PR_NOW)).toBe("3h");
    expect(age(pr.openedAt, PR_NOW - 160 * 60_000)).toBe("20m");
    expect(age(pr.openedAt, PR_NOW + 3 * 86_400_000)).toBe("3d");
    expect(age(undefined, PR_NOW)).toBe("");
  });
});

describe("checks and cleanup (decision 6, H-261 §15.6)", () => {
  it("keeps the head's checks and folds earlier commits", () => {
    const { head, earlier } = splitChecks(waitingPr());
    expect(head).toHaveLength(5);
    expect(earlier.map((c) => c.sha)).toEqual([OLDER]);
    expect(checksSummary(head).text).toBe("✓ 5 checks");
    expect(checksSummary(earlier).text).toBe("✕ 1 check failed");
  });

  it("says running and couldn't run", () => {
    expect(checksSummary([run(CheckResult.RUNNING)]).text).toBe("◌ Checks running");
    expect(checksSummary([run(CheckResult.ERROR), run(CheckResult.PASS)]).text).toBe(
      "⚠ 1 check couldn't run",
    );
    expect(checksSummary([]).text).toBe("○ No checks yet");
  });

  it("words cleanup as done, running or held", () => {
    const cleanup = mergedPr().cleanup;
    expect(cleanupLine(cleanup)?.text).toBe(
      "Cleaned up ✓ · 2 worktrees on mac, imac · 14.2 GB freed · branch deleted",
    );
    expect(cleanupLine(undefined)).toBeNull();
    const held = create(CleanupSchema, {
      state: CleanupState.HELD,
      items: [
        {
          jobId: "j",
          machine: "imac",
          kind: CleanupKind.WORKTREE,
          state: CleanupState.HELD,
          path: "wt",
          reason: "Uncommitted changes (salvaged)",
          salvaged: true,
        },
      ],
    });
    expect(cleanupLine(held)?.text).toBe("⏸ Cleanup held on imac: Uncommitted changes (salvaged)");
    expect(cleanupLine(create(CleanupSchema, { state: CleanupState.RUNNING }))?.text).toBe(
      "◌ Cleaning up…",
    );
  });
});
