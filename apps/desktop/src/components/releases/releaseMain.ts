// A release cut from main (H-278; UX-051 decision 10, H-261 §6.1): its tag
// line, what's in it, and what the owner's Leave out does. Pure functions
// over the release JSON and, where the service sends them, the PRs' own
// `hermes.pr.v1` messages, so the screen, the stories and the tests share them.

import type { PullRequest, Review } from "../../protocol/gen/hermes/pr/v1/pr_pb";
import { Verdict } from "../../protocol/gen/hermes/pr/v1/pr_pb";
import type { PrRef, Release } from "../../protocol/releases";
import { plural, releaseTitle } from "./labels";

/** The PRs' own records by number, where the service sends them (`pr_get`). */
export type PrRecords = ReadonlyMap<number, PullRequest>;

/** A chip's tone, as the release tones: ok (green), wait (amber), bad (red), off (dim). */
type ChipTone = "ok" | "wait" | "bad" | "off";

export interface Chip {
  readonly text: string;
  readonly tone: ChipTone;
}

const OWNER = "owner";

const ROLES: Readonly<Record<string, string>> = {
  architect: "Architect",
  ux: "UX",
  ce: "CE",
  devops: "DevOps",
  qa: "QA",
  owner: "You",
};

function roleName(role: string): string {
  return ROLES[role] ?? role.charAt(0).toUpperCase() + role.slice(1);
}

function short(sha: string): string {
  return sha.slice(0, 7);
}

/** Cut from main: the service sent the commit it was cut at. */
export function isFromMain(release: Release): boolean {
  return Boolean(release.commit);
}

/**
 * Whether the items carry their own Include / Leave out: while the owner
 * rules, and not when cut from main, where Leave out is per pull request
 * (UX-051 decision 10).
 */
export function itemsEditable(release: Release): boolean {
  return release.status === "awaiting_owner" && release.can_rule === true && !isFromMain(release);
}

/** "tag desktop-v0.18.0 on main at 9a1b2c3", and whether it is tagged yet. */
export function tagLine(release: Release): string {
  const name = release.tag || release.tag_name || `desktop-v${releaseTitle(release)}`;
  const at = `tag ${name} on main at ${short(release.commit ?? "")}`;
  return release.tag ? at : `${at} · DevOps tags it once you approve`;
}

/** The release whose commit starts this one's range, by name; else its commit. */
export function previousName(release: Release, all: readonly Release[]): string | null {
  const from = release.previous_commit;
  if (!from) {
    return null;
  }
  const before = all.find((r) => r.id !== release.id && r.commit === from);
  return before ? releaseTitle(before) : short(from);
}

/** Every PR the release lists, planned first. */
export function allPrs(release: Release): readonly PrRef[] {
  return [...(release.prs ?? []), ...(release.also_included ?? [])];
}

/** The latest review each role gave. */
function latest(pr: PullRequest): ReadonlyMap<string, Review> {
  const out = new Map<string, Review>();
  for (const r of pr.reviews) {
    out.set(r.role, r);
  }
  return out;
}

/** The roles a PR shows chips for: you first, then the required roles, then any other reviewer. */
function chipRoles(pr: PullRequest): readonly string[] {
  const waived = new Set(pr.waivers.map((w) => w.role));
  const reviewed = [...latest(pr).keys()];
  const roles = [
    ...(pr.ownerReviewRequired || reviewed.includes(OWNER) ? [OWNER] : []),
    ...pr.requiredRoles.filter((r) => r !== OWNER && !waived.has(r)),
    ...reviewed.filter((r) => r !== OWNER),
  ];
  return [...new Set(roles)];
}

/** One chip per role: "✓ Architect", "⟳ CE approved an older commit", "✕ …", "○ QA". */
export function reviewChips(pr: PullRequest): readonly Chip[] {
  const reviews = latest(pr);
  return chipRoles(pr).map((role) => {
    const who = roleName(role);
    const review = reviews.get(role);
    if (review?.verdict === Verdict.APPROVED) {
      return review.stale
        ? { text: `⟳ ${who} approved an older commit`, tone: "wait" }
        : { text: `✓ ${who}`, tone: "ok" };
    }
    if (review?.verdict === Verdict.CHANGES_REQUESTED) {
      return { text: `✕ ${who} asked for changes`, tone: "bad" };
    }
    return { text: `○ ${who}`, tone: "off" };
  });
}

/** Every role it needed approved it, at its own change. */
export function isReviewed(pr: PullRequest): boolean {
  const chips = reviewChips(pr);
  return chips.length > 0 && chips.every((c) => c.tone === "ok");
}

/**
 * "12 pull requests since 0.17.5 · all reviewed", or how many weren't. The
 * review clause needs every PR's record; without them the line stops at the count.
 */
export function summaryLine(release: Release, previous: string | null, records: PrRecords): string {
  const prs = allPrs(release);
  const count = previous
    ? `${plural(prs.length, "pull request")} since ${previous}`
    : `${plural(prs.length, "pull request")} on main`;
  const known = prs.map((p) => records.get(p.number));
  if (prs.length === 0 || known.some((pr) => pr === undefined)) {
    return count;
  }
  const unreviewed = known.filter((pr) => pr !== undefined && !isReviewed(pr)).length;
  return unreviewed === 0 ? `${count} · all reviewed` : `${count} · ${unreviewed} not reviewed`;
}

/** Statuses a Leave out is taken in, before the tag (H-272). */
const LEAVE_OUT: ReadonlySet<string> = new Set(["assembling", "built", "awaiting_owner", "held"]);

/** The PRs still in the release: not left out, and not an undo PR. */
function live(release: Release): readonly PrRef[] {
  return allPrs(release).filter((p) => !p.reverted && p.revert !== true);
}

/** Whether the owner can leave this PR out from here, now. */
export function canLeaveOut(release: Release, pr: PrRef): boolean {
  return (
    release.can_rule === true &&
    !release.tag &&
    LEAVE_OUT.has(release.status) &&
    live(release).length > 1 &&
    live(release).some((p) => p.number === pr.number)
  );
}

/** What a Leave out does: cut again before it, undo it on main, or (order unknown) either. */
export type LeaveOutPlan =
  | { readonly mode: "recut"; readonly commit: string }
  | { readonly mode: "revert" }
  | { readonly mode: "either" };

/**
 * The daemon cuts again when every left-out PR merged after every kept one,
 * and otherwise reverts. The planned list is in merge order, but its
 * interleaving with "Also included" isn't sent, so with both the plan may be "either".
 */
export function leaveOutPlan(release: Release, numbers: readonly number[]): LeaveOutPlan {
  if ((release.also_included ?? []).some((p) => !p.reverted && p.revert !== true)) {
    return { mode: "either" };
  }
  const prs = live(release);
  const left = (p: PrRef): boolean => numbers.includes(p.number);
  const kept = prs.reduce<PrRef | undefined>((last, p) => (left(p) ? last : p), undefined);
  const lastKept = kept ? prs.indexOf(kept) : -1;
  const firstLeft = prs.findIndex(left);
  return kept && firstLeft > lastKept
    ? { mode: "recut", commit: kept.merged_sha }
    : { mode: "revert" };
}

/** The Leave out confirmation's words: what happens to the release, the PR and its card. */
export function leaveOutBody(release: Release, pr: PrRef, plan: LeaveOutPlan): string {
  const version = releaseTitle(release);
  const again = "Its builds and tests start over, and you rule on the new build.";
  switch (plan.mode) {
    case "recut":
      return `${version} is cut again at ${short(plan.commit)}, just before #${pr.number}. The pull request stays on main and ships in a later release; ${pr.item_id} stays in Verify. ${again}`;
    case "revert":
      return `#${pr.number} merged before pull requests ${version} keeps, so it is undone on main: DevOps opens an undo pull request, which goes through the merge queue with its checks. When it merges, ${version} is cut again without it and ${pr.item_id} goes back to Doing. ${again}`;
    default:
      return `If #${pr.number} merged after everything ${version} keeps, ${version} is cut again just before it. Otherwise DevOps undoes it on main with a pull request through the merge queue, and ${pr.item_id} goes back to Doing. ${again}`;
  }
}

/** What the service did, for the toast. */
export function leaveOutDone(
  release: Release,
  pr: PrRef,
  result: { readonly mode: string; readonly commit?: string },
): string {
  return result.mode === "recut"
    ? `${releaseTitle(release)} is cut again at ${short(result.commit ?? "")} without #${pr.number}`
    : `DevOps is opening an undo pull request for #${pr.number}; it goes through the merge queue`;
}
