// The Pull requests tab's words (UX-051 decisions 2–4, 6, 11): every state is
// a glyph plus a word, never colour alone. Pure functions over the
// `hermes.pr.v1` messages, so the list, the detail and the tests share them.

import { timestampDate } from "@bufbuild/protobuf/wkt";
import type { Timestamp } from "@bufbuild/protobuf/wkt";
import type { Blocker, PullRequest, Review } from "../../protocol/gen/hermes/pr/v1/pr_pb";
import { BlockerKind, PrState, Verdict } from "../../protocol/gen/hermes/pr/v1/pr_pb";

/** How a state reads: ok (green), warn (amber), bad (red), you (accent), dim. */
export type Tone = "ok" | "warn" | "bad" | "you" | "dim";

export const OWNER_ROLE = "owner";

const ROLE_LABELS: Readonly<Record<string, string>> = {
  architect: "Architect",
  ux: "UX",
  ce: "CE",
  devops: "DevOps",
  qa: "QA",
  owner: "You",
};

/** "architect" → "Architect"; an unknown role keeps its name, capitalised. */
export function roleLabel(role: string): string {
  return ROLE_LABELS[role] ?? role.charAt(0).toUpperCase() + role.slice(1);
}

/** A commit as the screens show it: its first 7 characters. */
export function sha7(sha: string): string {
  return sha.slice(0, 7);
}

/** "40m", "3h", "2d": how long since `at`. */
export function age(at: Timestamp | undefined, now: number): string {
  if (at === undefined) {
    return "";
  }
  const minutes = Math.max(0, Math.floor((now - timestampDate(at).getTime()) / 60_000));
  if (minutes < 60) {
    return `${minutes}m`;
  }
  const hours = Math.floor(minutes / 60);
  return hours < 48 ? `${hours}h` : `${Math.floor(hours / 24)}d`;
}

const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/** "14:02" today, "8 Oct, 14:02" another day, in local time. */
export function clock(at: Timestamp | undefined, now: number): string {
  if (at === undefined) {
    return "";
  }
  const date = timestampDate(at);
  const time = `${String(date.getHours()).padStart(2, "0")}:${String(date.getMinutes()).padStart(2, "0")}`;
  return date.toDateString() === new Date(now).toDateString()
    ? time
    : `${date.getDate()} ${MONTHS[date.getMonth()] ?? ""}, ${time}`;
}

/** "you", "you, and CE", "you, CE, and checks" (UX-051 decision 4). */
export function joinNames(names: readonly string[]): string {
  if (names.length <= 1) {
    return names[0] ?? "";
  }
  return `${names.slice(0, -1).join(", ")}, and ${names.at(-1) ?? ""}`;
}

function blockers(pr: PullRequest): readonly Blocker[] {
  return pr.mergeable?.blockers ?? [];
}

function has(pr: PullRequest, kind: BlockerKind): boolean {
  return blockers(pr).some((b) => b.kind === kind);
}

const REVIEW_KINDS: ReadonlySet<BlockerKind> = new Set([
  BlockerKind.REVIEW_MISSING,
  BlockerKind.CHANGES_REQUESTED,
  BlockerKind.UNRESOLVED_MUST,
]);
const CHECK_KINDS: ReadonlySet<BlockerKind> = new Set([
  BlockerKind.CHECK_PENDING,
  BlockerKind.CHECK_FAILED,
]);

/** The branch must move first: a PR behind main or with conflicts doesn't wait for you (decision 11). */
const BRANCH_KINDS: ReadonlySet<BlockerKind> = new Set([
  BlockerKind.BEHIND_MAIN,
  BlockerKind.CONFLICTS,
  BlockerKind.MOVED_UNREPORTED,
]);

/** Whether the owner's review is what the PR waits on now: the reviewers are done (notes Q1). */
export function waitsForYou(pr: PullRequest): boolean {
  return (
    pr.state === PrState.OPEN &&
    has(pr, BlockerKind.OWNER_REVIEW) &&
    !blockers(pr).some(
      (b) => BRANCH_KINDS.has(b.kind) || (REVIEW_KINDS.has(b.kind) && b.subject !== OWNER_ROLE),
    )
  );
}

/** Who and what an open PR waits for, for "Waiting for: you, and CE." */
export function waitingFor(pr: PullRequest): string {
  const names: string[] = [];
  const add = (name: string): void => {
    if (!names.includes(name)) {
      names.push(name);
    }
  };
  if (has(pr, BlockerKind.OWNER_REVIEW)) {
    add("you");
  }
  for (const b of blockers(pr)) {
    if (REVIEW_KINDS.has(b.kind)) {
      add(b.subject === OWNER_ROLE ? "you" : roleLabel(b.subject));
    } else if (CHECK_KINDS.has(b.kind)) {
      add("checks");
    }
  }
  return joinNames(names);
}

/** What else holds a PR back, in the daemon's words: behind main, conflicts, the card. */
export function otherBlockers(pr: PullRequest): readonly Blocker[] {
  return blockers(pr).filter(
    (b) =>
      b.kind !== BlockerKind.OWNER_REVIEW &&
      b.kind !== BlockerKind.NOT_OPEN &&
      !REVIEW_KINDS.has(b.kind) &&
      !CHECK_KINDS.has(b.kind),
  );
}

/** The list's marker: conflicts first, then behind main (UX-051 decision 11). */
export function branchMarker(
  pr: PullRequest,
): { readonly text: string; readonly tone: Tone } | null {
  if (pr.state !== PrState.OPEN) {
    return null;
  }
  if (has(pr, BlockerKind.CONFLICTS)) {
    const author = pr.author?.name ?? "the author";
    return { text: `⚠ Has conflicts: waiting for ${author} to resolve`, tone: "warn" };
  }
  if (has(pr, BlockerKind.BEHIND_MAIN)) {
    return { text: "⇣ Needs update with main", tone: "dim" };
  }
  return null;
}

/** One reviewer's state, as a glyph, a word and the commit it names. */
export interface ReviewState {
  readonly glyph: string;
  /** The words after the glyph; `{sha}` marks where the commit goes. */
  readonly text: string;
  readonly sha: string;
  readonly tone: Tone;
}

/** "✓ Approved f0678fc" / "⟳ Approved c79c8b5, an older commit" / "✕ Asked for changes" / "○ Waiting". */
export function reviewState(
  pr: PullRequest,
  role: string,
  review: Review | undefined,
): ReviewState {
  if (review?.verdict === Verdict.APPROVED) {
    return review.stale
      ? { glyph: "⟳", text: "Approved {sha}, an older commit", sha: review.sha, tone: "warn" }
      : { glyph: "✓", text: "Approved {sha}", sha: review.sha, tone: "ok" };
  }
  if (review?.verdict === Verdict.CHANGES_REQUESTED) {
    return { glyph: "✕", text: "Asked for changes", sha: review.sha, tone: "bad" };
  }
  if (role === OWNER_ROLE && waitsForYou(pr)) {
    return { glyph: "▲", text: "Waiting for your review", sha: "", tone: "you" };
  }
  return { glyph: "○", text: "Waiting", sha: "", tone: "dim" };
}

/** The roles a PR shows rows for: you first, then the required roles, then anyone else who reviewed. */
export function reviewRoles(pr: PullRequest): readonly string[] {
  const roles = [...pr.requiredRoles];
  if (pr.ownerReviewRequired || pr.ownerFlagged) {
    roles.push(OWNER_ROLE);
  }
  for (const review of pr.reviews) {
    roles.push(review.role);
  }
  for (const waiver of pr.waivers) {
    roles.push(waiver.role);
  }
  const unique = [...new Set(roles)];
  return [...unique.filter((r) => r === OWNER_ROLE), ...unique.filter((r) => r !== OWNER_ROLE)];
}

export function reviewOf(pr: PullRequest, role: string): Review | undefined {
  return pr.reviews.find((review) => review.role === role);
}

/** A review chip for the list: "✓ Architect", "⟳ CE approved an older commit", "▲ You". */
export function reviewChip(
  pr: PullRequest,
  role: string,
): { readonly text: string; readonly tone: Tone } {
  const who = roleLabel(role);
  const state = reviewState(pr, role, reviewOf(pr, role));
  switch (state.glyph) {
    case "✓":
      return { text: `✓ ${who}`, tone: "ok" };
    case "⟳":
      return { text: `⟳ ${who} approved an older commit`, tone: "warn" };
    case "✕":
      return { text: `✕ ${who} asked for changes`, tone: "bad" };
    case "▲":
      return { text: "▲ You", tone: "you" };
    default:
      if (role !== OWNER_ROLE) {
        return { text: `○ ${who}`, tone: "dim" };
      }
      return {
        text: has(pr, BlockerKind.BEHIND_MAIN) ? "○ You after the update" : "○ You after reviewers",
        tone: "dim",
      };
  }
}

/** "✓ Merged into main · 14:02", "Merging into main…", "Closed without merging · 8 Oct, 12:20". */
export function stateLine(pr: PullRequest, now: number): string | null {
  switch (pr.state) {
    case PrState.MERGED:
      return `✓ Merged into ${pr.base || "main"} · ${clock(pr.mergedAt, now)}`;
    case PrState.MERGING:
      return `◌ Merging into ${pr.base || "main"}…`;
    case PrState.CLOSED:
      return `Closed without merging · ${clock(pr.closedAt, now)}`;
    default:
      return null;
  }
}
