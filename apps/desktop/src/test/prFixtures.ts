// Pull requests as UX-051's canvas shows them (#42 waiting for you, #43 behind
// main, #44 with conflicts, #41 merged), and a service that serves them.

import { create } from "@bufbuild/protobuf";
import type { MessageInitShape } from "@bufbuild/protobuf";
import { timestampFromDate } from "@bufbuild/protobuf/wkt";
import type { PrDiff, PullRequest } from "../protocol/gen/hermes/pr/v1/pr_pb";
import {
  BlockerKind,
  CheckResult,
  CleanupKind,
  CleanupState,
  FileStatus,
  MergeableSchema,
  PrDiffSchema,
  PrListSchema,
  PrState,
  PullRequestSchema,
  Severity,
  Verdict,
} from "../protocol/gen/hermes/pr/v1/pr_pb";
import { PULL_REQUESTS } from "../protocol/prs";
import { cardsDaemon } from "./cardFixtures";
import type { FakeDaemon } from "./fakeDaemon";

/** 12:00 UTC on 9 Oct 2026: what stories and tests take as now. */
export const PR_NOW = Date.UTC(2026, 9, 9, 12, 0);

export const HEAD = "f0678fc1a2b3c4d5e6f708192a3b4c5d6e7f8091";
export const OLDER = "c79c8b5a1b2c3d4e5f60718293a4b5c6d7e8f901";

function at(minutesAgo: number) {
  return timestampFromDate(new Date(PR_NOW - minutesAgo * 60_000));
}

function bot(name: string) {
  return { daemonId: "d-mac", botId: `b-${name.toLowerCase().replaceAll(" ", "-")}`, name };
}

function approved(role: string, name: string, sha: string, minutesAgo: number, summary = "") {
  return {
    role,
    reviewer: { who: { case: "bot" as const, value: bot(name) } },
    sha,
    verdict: Verdict.APPROVED,
    summary,
    stale: sha !== HEAD,
    at: at(minutesAgo),
  };
}

function check(name: string, result: CheckResult, ranOn = "mac", sha = HEAD, minutes = 4) {
  return {
    name,
    sha,
    result,
    ranOn,
    required: true,
    ...(result === CheckResult.PASS || result === CheckResult.FAIL
      ? { startedAt: at(60), finishedAt: at(60 - minutes) }
      : {}),
  };
}

const PASSED = [
  check("Rust format, lint, 400-line limit", CheckResult.PASS, "mac", HEAD, 2),
  check("Rust tests · 1223 passed", CheckResult.PASS, "mac", HEAD, 6),
  check("Desktop tests · 1076 passed", CheckResult.PASS),
  check("Visual regression · 136 of 136", CheckResult.PASS, "mac", HEAD, 5),
  check("Windows build", CheckResult.PASS, "win-pc", HEAD, 9),
  check("Desktop tests · 2 timeouts", CheckResult.FAIL, "mac", OLDER),
];

function pr(init: Exclude<MessageInitShape<typeof PullRequestSchema>, PullRequest>): PullRequest {
  return create(PullRequestSchema, {
    projectId: "p1",
    base: "main",
    headSha: HEAD,
    state: PrState.OPEN,
    author: bot("Desktop Dev"),
    openedAt: at(180),
    ...init,
  });
}

/** #42: the reviewers are in, CE approved an older commit, and it waits for you. */
export function waitingPr(): PullRequest {
  return pr({
    number: 42,
    itemId: "H-293",
    itemTitle: "Waiting for you on releases",
    branch: "H-247-waiting-for-you",
    changeNote:
      "A release now says what it waits on from you: Run cards, decisions, questions and your ruling, each with its button. See H-292.\n\nTested: 1223 Rust, 1076 desktop, VR 136/136.",
    reviews: [
      approved("architect", "Architect", HEAD, 25, "M1 and M2 fixed."),
      {
        ...approved("ux", "UX Designer", HEAD, 20, "Review… focuses Approve or the heading."),
        findings: [
          { severity: Severity.MUST, text: "Review… focuses Reject", resolvedIn: HEAD },
          {
            severity: Severity.SHOULD,
            text: "N+1 reads in owner_blockers",
            followUpItemId: "H-292",
          },
        ],
      },
      approved("ce", "Context Engineer", OLDER, 120),
    ],
    checks: PASSED,
    requiredRoles: ["architect", "ux", "ce"],
    ownerReviewRequired: true,
    mergeable: {
      blockers: [
        { kind: BlockerKind.REVIEW_MISSING, subject: "ce", text: "CE approved an older commit" },
        {
          kind: BlockerKind.OWNER_REVIEW,
          subject: "owner",
          text: "Required: this project reviews every PR",
        },
      ],
    },
    docsChanged: true,
    filesChanged: 9,
    additions: 412,
    deletions: 58,
    commentCount: 3,
  });
}

/** #42 once CE re-approved: only you are missing. */
export function readyForYouPr(): PullRequest {
  const ready = waitingPr();
  for (const review of ready.reviews) {
    review.sha = HEAD;
    review.stale = false;
  }
  ready.mergeable = create(MergeableSchema, {
    blockers: [{ kind: BlockerKind.OWNER_REVIEW, subject: "owner", text: "Your review" }],
  });
  return ready;
}

function behindPr(): PullRequest {
  return pr({
    number: 43,
    itemId: "H-292",
    itemTitle: "Waiting for you on the iPhone",
    branch: "H-248-waiting",
    author: bot("iOS Dev iMac"),
    openedAt: at(60),
    reviews: [approved("ux", "UX Designer", HEAD, 60)],
    checks: [
      check("iOS unit tests", CheckResult.RUNNING),
      check("UI tests", CheckResult.QUEUED, ""),
    ],
    requiredRoles: ["ux"],
    ownerReviewRequired: true,
    mergeable: {
      blockers: [
        { kind: BlockerKind.BEHIND_MAIN, text: "Needs update with main" },
        { kind: BlockerKind.CHECK_PENDING, subject: "iOS unit tests", text: "running" },
        { kind: BlockerKind.OWNER_REVIEW, subject: "owner", text: "Your review" },
      ],
    },
    filesChanged: 6,
    additions: 301,
    deletions: 22,
  });
}

function conflictPr(): PullRequest {
  return pr({
    number: 44,
    itemId: "H-500",
    itemTitle: "Faster board loads",
    branch: "H-253-board-loads",
    author: bot("Backend Dev"),
    openedAt: at(40),
    reviews: [
      {
        ...approved("architect", "Architect", HEAD, 30, "Keep the cache bounded."),
        verdict: Verdict.CHANGES_REQUESTED,
      },
    ],
    checks: [check("Rust tests", CheckResult.RUNNING)],
    requiredRoles: ["architect"],
    ownerReviewRequired: true,
    mergeable: {
      blockers: [
        {
          kind: BlockerKind.CHANGES_REQUESTED,
          subject: "architect",
          text: "Architect asked for changes",
        },
        {
          kind: BlockerKind.CONFLICTS,
          text: "Has conflicts: waiting for Backend Dev to resolve",
          paths: ["DocsView.tsx", "docs.css"],
        },
        { kind: BlockerKind.OWNER_REVIEW, subject: "owner", text: "Your review" },
      ],
    },
    filesChanged: 3,
    additions: 80,
    deletions: 120,
  });
}

/** #41: merged at 09:12, its cleanup done. */
export function mergedPr(): PullRequest {
  return pr({
    number: 41,
    itemId: "H-293",
    itemTitle: "Install on this iPhone",
    branch: "H-230-install",
    author: bot("iOS Dev iMac"),
    state: PrState.MERGED,
    mergedAt: at(168),
    mergedSha: HEAD,
    reviews: [
      {
        role: "owner",
        reviewer: { who: { case: "owner", value: { deviceId: "dev-1", deviceName: "iPhone" } } },
        sha: HEAD,
        verdict: Verdict.APPROVED,
        at: at(170),
      },
      approved("ce", "Context Engineer", HEAD, 200),
      approved("ux", "UX Designer", HEAD, 210),
    ],
    checks: [check("iOS unit tests", CheckResult.PASS, "imac")],
    requiredRoles: ["ce", "ux", "owner"],
    ownerReviewRequired: true,
    mergeable: { blockers: [{ kind: BlockerKind.NOT_OPEN, text: "Merged" }] },
    cleanup: {
      state: CleanupState.DONE,
      freedBytes: 14_200_000_000n,
      branchDeleted: true,
      items: [
        {
          jobId: "j1",
          machine: "mac",
          kind: CleanupKind.WORKTREE,
          state: CleanupState.DONE,
          path: "gravity-wt-h230",
          freedBytes: 3_100_000_000n,
        },
        {
          jobId: "j2",
          machine: "imac",
          kind: CleanupKind.WORKTREE,
          state: CleanupState.DONE,
          path: "gravity-wt-h230-ios",
          freedBytes: 11_100_000_000n,
        },
      ],
    },
  });
}

export function prList(): PullRequest[] {
  return [readyForYouPr(), behindPr(), conflictPr(), mergedPr()];
}

const DIFF = [
  "diff --git a/apps/desktop/src/components/releases/ReleaseReview.tsx b/apps/desktop/src/components/releases/ReleaseReview.tsx",
  "--- a/apps/desktop/src/components/releases/ReleaseReview.tsx",
  "+++ b/apps/desktop/src/components/releases/ReleaseReview.tsx",
  "@@ -48,4 +48,5 @@ export function focusReview",
  " export function focusReview(root: HTMLElement | null): void {",
  '-  const target = root?.querySelector(".release-bar button:not([disabled])");',
  '+  const approve = root?.querySelector(".release-bar .btn-primary:not([disabled])");',
  '+  const target = approve ?? root?.querySelector(".release-head h2");',
  "   target?.focus({ preventScroll: true });",
  " }",
  "diff --git a/docs/user/releases.md b/docs/user/releases.md",
  "--- a/docs/user/releases.md",
  "+++ b/docs/user/releases.md",
  "@@ -1,2 +1,3 @@",
  " # Releases",
  "+**Waiting for you.** A release lists what only you can do.",
  " A release is a tag of main.",
  "",
].join("\n");

export function prDiff(fromSha = "1bb039a1b2c3d4e5f60718293a4b5c6d7e8f9012"): PrDiff {
  return create(PrDiffSchema, {
    fromSha,
    toSha: HEAD,
    files: [
      {
        path: "apps/desktop/src/components/releases/ReleaseReview.tsx",
        status: FileStatus.MODIFIED,
        additions: 2,
        deletions: 1,
      },
      { path: "docs/user/releases.md", status: FileStatus.MODIFIED, additions: 1 },
      { path: "apps/desktop/tests/visual/release.png", status: FileStatus.MODIFIED, binary: true },
    ],
    diff: DIFF,
  });
}

/** A service that serves pull requests (the list, each PR by number, diffs) and card links. */
export function prDaemon(
  prs: readonly PullRequest[] = prList(),
  /** What `pr_get` answers instead, by number. */
  details: readonly PullRequest[] = [],
): FakeDaemon {
  const fake = cardsDaemon();
  fake.capabilities = [...fake.capabilities, PULL_REQUESTS];
  const byNumber = new Map([...prs, ...details].map((p) => [p.number, p]));
  fake.onPr("prList", () => ({ case: "prList", value: create(PrListSchema, { prs: [...prs] }) }));
  fake.onPr("prGet", (call) => {
    const found = byNumber.get(call.case === "prGet" ? (call.value.number ?? 0) : 0);
    if (found === undefined) {
      throw new Error("not_found");
    }
    return { case: "pr", value: found };
  });
  fake.onPr("prDiff", (call) => ({
    case: "prDiff",
    value: prDiff(call.case === "prDiff" ? call.value.fromSha : undefined),
  }));
  return fake;
}
