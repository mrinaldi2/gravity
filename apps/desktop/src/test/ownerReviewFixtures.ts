// The owner's review (H-277): #42 ready for you, in its 10 s window and past
// it; #45 to re-check after conflict fixes; line comments on #42's diff; and
// a service that takes the owner's writes over the app's own connection.

import { create } from "@bufbuild/protobuf";
import type { MessageInitShape } from "@bufbuild/protobuf";
import { timestampFromDate } from "@bufbuild/protobuf/wkt";
import type { LineComment, PullRequest } from "../protocol/gen/hermes/pr/v1/pr_pb";
import {
  BlockerKind,
  LineCommentSchema,
  MergeableSchema,
  PrCommentsSchema,
  PrState,
  ReviewSchema,
  Severity,
  Side,
  Verdict,
} from "../protocol/gen/hermes/pr/v1/pr_pb";
import type { FakeDaemon } from "./fakeDaemon";
import { HEAD, OLDER, PR_NOW, prDaemon, prList, readyForYouPr } from "./prFixtures";

const FILE = "apps/desktop/src/components/releases/ReleaseReview.tsx";

function ownerReview(sha: string, minutesAgo: number) {
  return create(ReviewSchema, {
    role: "owner",
    reviewer: { who: { case: "owner", value: { deviceId: "", deviceName: "" } } },
    sha,
    verdict: Verdict.APPROVED,
    stale: sha !== HEAD,
    at: timestampFromDate(new Date(PR_NOW - minutesAgo * 60_000)),
  });
}

/** #42 after your Approve: merging, with `seconds` left of the Undo window (negative: past it). */
export function mergingPr(seconds: number): PullRequest {
  const merging = readyForYouPr();
  merging.reviews.push(ownerReview(HEAD, 0));
  merging.state = PrState.MERGING;
  merging.mergeAt = timestampFromDate(new Date(PR_NOW + seconds * 1000));
  merging.mergeable = create(MergeableSchema, { ok: true });
  return merging;
}

/** #45: you approved c79c8b5, the author fixed conflicts since, and the bots re-checked. */
export function recheckPr(): PullRequest {
  const recheck = readyForYouPr();
  recheck.number = 45;
  recheck.itemId = "H-259";
  recheck.itemTitle = "Search in Docs";
  recheck.reviews.push(ownerReview(OLDER, 90));
  return recheck;
}

/** #46: CE hasn't approved yet, but a stale Needs-you row still names it. */
export function botsPendingPr(): PullRequest {
  const pending = readyForYouPr();
  pending.number = 46;
  pending.mergeable = create(MergeableSchema, {
    blockers: [
      { kind: BlockerKind.REVIEW_MISSING, subject: "ce", text: "Waiting for CE" },
      { kind: BlockerKind.OWNER_REVIEW, subject: "owner", text: "Your review" },
    ],
  });
  return pending;
}

function comment(
  init: Exclude<MessageInitShape<typeof LineCommentSchema>, LineComment>,
): LineComment {
  return create(LineCommentSchema, {
    sha: HEAD,
    path: FILE,
    side: Side.NEW,
    author: {
      who: { case: "bot", value: { daemonId: "d-mac", botId: "b-ux", name: "UX Designer" } },
    },
    ...init,
  });
}

/** #42's threads: one open with a reply on line 50, one resolved, one outdated. */
function prComments(): LineComment[] {
  return [
    comment({
      id: "c1",
      line: 50,
      body: "Good: never lands on Reject.",
      severity: Severity.SHOULD,
    }),
    comment({ id: "c2", line: 50, replyTo: "c1", body: "Kept the heading's tabIndex -1." }),
    comment({ id: "c3", line: 49, body: "Name it approve.", resolved: true }),
    comment({ id: "c4", line: 12, body: "This helper moved.", outdated: true }),
  ];
}

/** A service with PRs, comments and every owner write recorded in `requests`. */
export function ownerDaemon(
  details: readonly PullRequest[] = [readyForYouPr()],
  list: readonly PullRequest[] = prList(),
): FakeDaemon {
  const fake = prDaemon(list, details);
  fake.grants = ["read", "control", "approve"];
  fake.onPr("prComments", () => ({
    case: "prComments",
    value: create(PrCommentsSchema, { sha: HEAD, comments: prComments() }),
  }));
  const settings = { project_id: "p1", owner_review: "all", owner_review_areas: [], areas: [] };
  fake.onRequest("review_settings_get", () => ({
    type: "review_settings",
    req_id: "r",
    review_settings: { ...settings, areas: ["security", "releases", "docs", "desktop"] },
  }));
  fake.onRequest("review_settings_set", (body) => ({
    type: "review_settings",
    req_id: "r",
    review_settings:
      body.type === "review_settings_set"
        ? {
            ...settings,
            owner_review: body.owner_review,
            owner_review_areas: [...body.owner_review_areas],
            areas: ["security", "releases", "docs", "desktop"],
          }
        : settings,
  }));
  for (const type of ["pr_review_submit", "pr_merge_undo"] as const) {
    fake.onRequest(type, () => ({ type: "pr", req_id: "r", pr: {} }));
  }
  for (const type of ["pr_comment_add", "pr_comment_resolve"] as const) {
    fake.onRequest(type, () => ({ type: "comment", req_id: "r", comment: {} }));
  }
  return fake;
}
