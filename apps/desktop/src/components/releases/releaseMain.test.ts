import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { fromJson } from "@bufbuild/protobuf";
import type { JsonValue } from "@bufbuild/protobuf";
import { describe, expect, it } from "vitest";
import type { PullRequest } from "../../protocol/gen/hermes/pr/v1/pr_pb";
import { PullRequestSchema } from "../../protocol/gen/hermes/pr/v1/pr_pb";
import type { PrRef, Release } from "../../protocol/releases";
import {
  MAIN_RECORDS,
  PREVIOUS,
  mainRelease,
  previousRelease,
} from "../../test/releaseMainFixtures";
import {
  canLeaveOut,
  isReviewed,
  leaveOutBody,
  leaveOutDone,
  leaveOutPlan,
  previousName,
  reviewChips,
  summaryLine,
  tagLine,
} from "./releaseMain";

/** The PR-0 fixtures the daemon's contract tests use (H-265). */
function busFixture(name: string): JsonValue {
  const repo = join(dirname(fileURLToPath(import.meta.url)), "..", "..", "..", "..", "..");
  return JSON.parse(
    readFileSync(join(repo, "crates", "bus", "fixtures", "pr", `${name}.json`), "utf8"),
  ) as JsonValue;
}

const pr = (name: string): PullRequest =>
  fromJson(PullRequestSchema, busFixture(name), { ignoreUnknownFields: true });

const texts = (p: PullRequest): readonly string[] => reviewChips(p).map((c) => c.text);

/** 0.18.0 with the H-265 `release_from_main.json` keys as the service sends them. */
const fromFixture = (): Release =>
  mainRelease({ ...(busFixture("release_from_main") as Partial<Release>) });

const ref = (r: Release, n: number): PrRef => {
  const found = [...(r.prs ?? []), ...(r.also_included ?? [])].find((p) => p.number === n);
  if (!found) {
    throw new Error(`no #${n}`);
  }
  return found;
};

describe("review chips, on the H-265 fixtures", () => {
  it("shows you first, then each required role, all approved", () => {
    expect(texts(pr("pr_merged"))).toEqual(["✓ You", "✓ Architect", "✓ UX"]);
    expect(isReviewed(pr("pr_merged"))).toBe(true);
  });

  it("names an older approval, a change request and a missing review", () => {
    expect(texts(pr("pr_open"))).toEqual([
      "○ You",
      "⟳ Architect approved an older commit",
      "✓ UX",
      "✕ CE asked for changes",
    ]);
    expect(reviewChips(pr("pr_open")).map((c) => c.tone)).toEqual(["off", "wait", "ok", "bad"]);
    expect(isReviewed(pr("pr_open"))).toBe(false);
  });

  it("leaves a waived role out, and carries an approval of the same change", () => {
    expect(texts(pr("pr_behind"))).toEqual(["✓ Architect"]);
  });

  it("shows a role that hasn't reviewed as waiting, so the PR isn't reviewed", () => {
    expect(texts(pr("pr_closed"))).toEqual(["○ Architect", "○ CE"]);
    expect(isReviewed(pr("pr_closed"))).toBe(false);
  });
});

describe("the release's lines", () => {
  it("names the tag and commit, and that it's tagged once you approve", () => {
    expect(tagLine(fromFixture())).toBe("tag desktop-v0.18.0 on main at 9a1b2c3");
    expect(tagLine(mainRelease())).toBe(
      "tag desktop-v0.18.0 on main at 9a1b2c3 · DevOps tags it once you approve",
    );
  });

  it("finds the previous release by its commit, else shows the commit", () => {
    const r = mainRelease();
    expect(previousName(r, [r, previousRelease()])).toBe("0.17.5");
    expect(previousName(r, [r])).toBe(PREVIOUS.slice(0, 7));
    expect(previousName(mainRelease({ previous_commit: null }), [r])).toBeNull();
  });

  it("counts the PRs since the previous release, and says whether all were reviewed", () => {
    const r = fromFixture();
    expect(summaryLine(r, "0.17.5", new Map())).toBe("3 pull requests since 0.17.5");
    expect(summaryLine(r, "0.17.5", MAIN_RECORDS)).toBe(
      "3 pull requests since 0.17.5 · all reviewed",
    );
    const some = new Map(MAIN_RECORDS).set(42, pr("pr_open"));
    expect(summaryLine(r, "0.17.5", some)).toBe("3 pull requests since 0.17.5 · 1 not reviewed");
    expect(summaryLine(r, null, MAIN_RECORDS)).toBe("3 pull requests on main · all reviewed");
  });
});

/** 0.18.0 with only planned PRs, so their merge order is known. */
const planned = (): Release => mainRelease({ also_included: [] });

describe("Leave out", () => {
  it("is the owner's, before the tag, while the package can start over", () => {
    const r = mainRelease();
    expect(canLeaveOut(r, ref(r, 42))).toBe(true);
    expect(canLeaveOut({ ...r, can_rule: false }, ref(r, 42))).toBe(false);
    expect(canLeaveOut({ ...r, tag: "desktop-v0.18.0" }, ref(r, 42))).toBe(false);
    expect(canLeaveOut({ ...r, status: "deploying" }, ref(r, 42))).toBe(false);
    const left = mainRelease({
      prs: [{ ...ref(r, 40), reverted: true }, ref(r, 42)],
      also_included: [],
    });
    expect(canLeaveOut(left, ref(left, 40))).toBe(false);
    // The last PR in it can't go: cancel the release instead.
    expect(canLeaveOut(left, ref(left, 42))).toBe(false);
  });

  it("cuts again before the last PRs, and undoes an earlier one on main", () => {
    const r = planned();
    expect(leaveOutPlan(r, [42])).toEqual({ mode: "recut", commit: ref(r, 40).merged_sha });
    expect(leaveOutPlan(r, [40])).toEqual({ mode: "revert" });
    // Its interleaving with Also included isn't sent: either outcome.
    expect(leaveOutPlan(mainRelease(), [42])).toEqual({ mode: "either" });
  });

  it("says what happens to the release, the PR and its card", () => {
    const r = planned();
    expect(leaveOutBody(r, ref(r, 42), leaveOutPlan(r, [42]))).toBe(
      "0.18.0 is cut again at 9a1b2c3, just before #42. The pull request stays on main and ships in a later release; H-247 stays in Verify. Its builds and tests start over, and you rule on the new build.",
    );
    expect(leaveOutBody(r, ref(r, 40), leaveOutPlan(r, [40]))).toContain(
      "DevOps opens an undo pull request, which goes through the merge queue with its checks. When it merges, 0.18.0 is cut again without it and H-203 goes back to Doing.",
    );
    expect(leaveOutBody(r, ref(r, 40), { mode: "either" })).toContain("If #40 merged after");
    expect(leaveOutDone(r, ref(r, 42), { mode: "recut", commit: ref(r, 40).merged_sha })).toBe(
      "0.18.0 is cut again at 9a1b2c3 without #42",
    );
    expect(leaveOutDone(r, ref(r, 40), { mode: "revert" })).toContain("undo pull request for #40");
  });
});
