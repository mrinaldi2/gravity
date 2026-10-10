// The owner's review in words (UX-051 decisions 5, 8 and 11; H-261 §4.3, §16):
// what Approve and Ask for changes do, the 10 s Undo, the Re-check of a
// change since you approved, and the Owner review setting. Pure, so the PR,
// Needs you and the tests share them.

import { timestampDate } from "@bufbuild/protobuf/wkt";
import type { PullRequest, ReviewSettings } from "../../protocol/gen/hermes/pr/v1/pr_pb";
import { BlockerKind, OwnerReview, PrState, Verdict } from "../../protocol/gen/hermes/pr/v1/pr_pb";
import type { OwnerReviewMode } from "../../protocol/prOwner";
import { checksSummary, splitChecks } from "./checkText";
import { joinNames, OWNER_ROLE, reviewOf, roleLabel, sha7, waitingFor } from "./prText";

/** "the next release" is where a merged PR goes (ruling fc46b042). */
const SHIPS = "merges into main and ships in the next release";

/** Who else the PR waits for once you approve: "CE", "CE, and checks", or "". */
function othersAfterYou(pr: PullRequest): string {
  const names = waitingFor(pr)
    .split(/, and |, /)
    .filter((name) => name !== "" && name !== "you");
  return joinNames(names);
}

/** What Approve triggers, said on the button's choice (decision 5). */
export function approveCopy(pr: PullRequest): string {
  const others = othersAfterYou(pr);
  if (others === "") {
    return `#${pr.number} ${SHIPS}. You have 10 seconds to undo.`;
  }
  const verb = others === "checks" ? "pass" : others.includes(", and ") ? "approve" : "approves";
  return `When ${others} ${verb} too, #${pr.number} ${SHIPS}.`;
}

/** What Ask for changes does. */
export function changesCopy(pr: PullRequest): string {
  return `${pr.author?.name ?? "The author"} gets your note; the card goes back to Doing.`;
}

/** Why Approve is off, or null when it's on: only the latest, reported commit (decision 5). */
export function approveBlocked(pr: PullRequest, reviewing: string): string | null {
  if (pr.state !== PrState.OPEN) {
    return "This pull request isn't open.";
  }
  if (pr.movedUnreported) {
    return "The branch moved without a reported push. Wait for the author to report it.";
  }
  if (reviewing !== pr.headSha) {
    return `New commits came in: ${sha7(pr.headSha)} is the latest. Review it again.`;
  }
  return null;
}

/** The owner's approval of an older change, whose delta a Re-check shows (decision 11). */
export function recheckBase(pr: PullRequest): string | null {
  const mine = reviewOf(pr, OWNER_ROLE);
  return mine?.verdict === Verdict.APPROVED && mine.stale ? mine.sha : null;
}

/** The 10 s Undo, then waiting for DevOps (§16; H-271, H-284 S3). */
export type MergeWindow =
  | { readonly kind: "undo"; readonly seconds: number }
  | { readonly kind: "devops" }
  | null;

export function mergeWindow(pr: PullRequest, now: number): MergeWindow {
  if (pr.state !== PrState.MERGING || pr.mergeAt === undefined) {
    return null;
  }
  const left = timestampDate(pr.mergeAt).getTime() - now;
  return left > 0 ? { kind: "undo", seconds: Math.ceil(left / 1000) } : { kind: "devops" };
}

/** Whether your fresh approval is what an Undo withdraws. */
export function youApproved(pr: PullRequest): boolean {
  const mine = reviewOf(pr, OWNER_ROLE);
  return mine?.verdict === Verdict.APPROVED && !mine.stale && mine.sha === pr.headSha;
}

/** Whether every required bot role approved the current change: your row comes after (ruling 7629a873). */
export function botsDone(pr: PullRequest): boolean {
  return !(pr.mergeable?.blockers ?? []).some(
    (b) =>
      (b.kind === BlockerKind.REVIEW_MISSING || b.kind === BlockerKind.CHANGES_REQUESTED) &&
      b.subject !== OWNER_ROLE,
  );
}

/** "Desktop Dev · Architect ✓ UX ✓ · checks ✓ · merges into main when you approve". */
export function reviewRowMeta(pr: PullRequest): string {
  const approved = pr.reviews
    .filter((r) => r.role !== OWNER_ROLE && r.verdict === Verdict.APPROVED && !r.stale)
    .map((r) => `${roleLabel(r.role)} ✓`);
  const checks = checksSummary(splitChecks(pr).head).text;
  const others = othersAfterYou(pr);
  const when =
    others === "" ? "merges into main when you approve" : `waits for ${others} after you`;
  return [pr.author?.name ?? "", approved.join(" "), checks, when]
    .filter((part) => part !== "")
    .join(" · ");
}

const MODES: Readonly<Record<OwnerReview, OwnerReviewMode>> = {
  [OwnerReview.UNSPECIFIED]: "all",
  [OwnerReview.ALL]: "all",
  [OwnerReview.AREAS]: "areas",
  [OwnerReview.FLAGGED]: "flagged",
  [OwnerReview.NONE]: "none",
};

export function modeOf(settings: ReviewSettings): OwnerReviewMode {
  return MODES[settings.ownerReview];
}

/** The setting's choices, in the canvas's words (decision 8). */
export const MODE_CHOICES: readonly {
  readonly mode: OwnerReviewMode;
  readonly title: string;
  readonly hint: string;
}[] = [
  {
    mode: "all",
    title: "Every pull request",
    hint: "Nothing merges to main without you. Start here.",
  },
  { mode: "areas", title: "Some areas", hint: "Only PRs that touch an area you pick:" },
  {
    mode: "flagged",
    title: "Only flagged",
    hint: "A reviewer or the lead flags a PR for you, with a reason.",
  },
  {
    mode: "none",
    title: "None",
    hint: "Reviewers' approvals merge PRs. You see them in each release.",
  },
];

/** reviewers.toml's area keys in the glossary's words (UX-051; UX-053 must-fix 1). */
const AREA_LABELS: ReadonlyMap<string, string> = new Map([
  ["security", "Security and permissions"],
  ["releases", "Releases and installs"],
  ["docs", "Docs"],
  ["desktop", "Desktop screens"],
  ["ios", "iPhone and iPad screens"],
  ["service", "Hermes service"],
]);

/** An area's name: "Security and permissions" for `security`; an unknown key as it is. */
export function areaLabel(key: string): string {
  return AREA_LABELS.get(key) ?? key;
}

/** "Owner review: every pull request", shown on each PR's Waiting for line. */
export function settingLine(settings: ReviewSettings): string {
  switch (modeOf(settings)) {
    case "areas":
      return settings.ownerReviewAreas.length === 0
        ? "Owner review: some areas (none picked)"
        : `Owner review: some areas (${settings.ownerReviewAreas.map(areaLabel).join(", ")})`;
    case "flagged":
      return "Owner review: only flagged";
    case "none":
      return "Owner review: none";
    default:
      return "Owner review: every pull request";
  }
}
