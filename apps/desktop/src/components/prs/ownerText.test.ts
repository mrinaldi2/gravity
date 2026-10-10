import { create } from "@bufbuild/protobuf";
import { describe, expect, it } from "vitest";
import { OwnerReview, PrState, ReviewSettingsSchema } from "../../protocol/gen/hermes/pr/v1/pr_pb";
import { decodeReviewSettings } from "../../protocol/prOwner";
import { mergingPr, recheckPr } from "../../test/ownerReviewFixtures";
import { HEAD, OLDER, PR_NOW, readyForYouPr, waitingPr } from "../../test/prFixtures";
import {
  approveBlocked,
  approveCopy,
  mergeWindow,
  recheckBase,
  settingLine,
  youApproved,
} from "./ownerText";

function line(ownerReview: OwnerReview): string {
  return settingLine(create(ReviewSettingsSchema, { ownerReview }));
}

describe("ownerText", () => {
  it("says what Approve triggers", () => {
    expect(approveCopy(readyForYouPr())).toBe(
      "#42 merges into main and ships in the next release. You have 10 seconds to undo.",
    );
    expect(approveCopy(waitingPr())).toBe(
      "When CE approves too, #42 merges into main and ships in the next release.",
    );
  });

  it("allows Approve only on the latest, reported commit of an open PR", () => {
    const pr = readyForYouPr();
    expect(approveBlocked(pr, HEAD)).toBeNull();
    expect(approveBlocked(pr, OLDER)).toBe(
      "New commits came in: f0678fc is the latest. Review it again.",
    );
    pr.movedUnreported = true;
    expect(approveBlocked(pr, HEAD)).toMatch(/moved without a reported push/);
    pr.state = PrState.MERGED;
    expect(approveBlocked(pr, HEAD)).toBe("This pull request isn't open.");
  });

  it("finds your older approval to re-check from", () => {
    expect(recheckBase(recheckPr())).toBe(OLDER);
    expect(recheckBase(readyForYouPr())).toBeNull();
  });

  it("counts the Undo window down, then waits for DevOps", () => {
    expect(mergeWindow(mergingPr(7), PR_NOW)).toEqual({ kind: "undo", seconds: 7 });
    expect(mergeWindow(mergingPr(7), PR_NOW + 6_500)).toEqual({ kind: "undo", seconds: 1 });
    expect(mergeWindow(mergingPr(7), PR_NOW + 7_000)).toEqual({ kind: "devops" });
    expect(mergeWindow(readyForYouPr(), PR_NOW)).toBeNull();
    expect(youApproved(mergingPr(7))).toBe(true);
    expect(youApproved(recheckPr())).toBe(false);
  });

  it("reads and words the Owner review setting", () => {
    const areas = decodeReviewSettings({
      project_id: "p1",
      owner_review: "areas",
      owner_review_areas: ["security", "docs"],
      areas: ["security", "docs", "desktop"],
    });
    expect(areas.ownerReview).toBe(OwnerReview.AREAS);
    expect(areas.areas).toEqual(["security", "docs", "desktop"]);
    expect(settingLine(areas)).toBe("Owner review: some areas (security, docs)");
    expect(line(OwnerReview.ALL)).toBe("Owner review: every pull request");
    expect(line(OwnerReview.UNSPECIFIED)).toBe("Owner review: every pull request");
    expect(line(OwnerReview.FLAGGED)).toBe("Owner review: only flagged");
    expect(line(OwnerReview.NONE)).toBe("Owner review: none");
  });
});
