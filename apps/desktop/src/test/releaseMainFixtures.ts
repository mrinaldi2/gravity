// A release cut from main, for tests and stories (H-278): the PRs of the
// H-265 fixture `crates/bus/fixtures/pr/release_from_main.json`, plus the
// keys H-272 adds, and the PRs' own records for their review chips.

import { create } from "@bufbuild/protobuf";
import type { PullRequest } from "../protocol/gen/hermes/pr/v1/pr_pb";
import { PrState, PullRequestSchema, Verdict } from "../protocol/gen/hermes/pr/v1/pr_pb";
import type { Release } from "../protocol/releases";
import { release } from "./releaseFixtures";

/** The commit 0.18.0 is cut at, and the one 0.17.5 was tagged at. */
const CUT = "9a1b2c3d4e5f60718293a4b5c6d7e8f901234567";
export const PREVIOUS = "5c4b3a2918f7e6d5c4b3a2918f7e6d5c4b3a2918";

export const MAIN_TITLES: ReadonlyMap<string, string> = new Map([
  ["H-203", "Card ids as links"],
  ["H-247", "Waiting for you on releases"],
  ["H-250", "Typo in the About box"],
  ["H-259", "Docs tab"],
]);

/** 0.18.0 awaiting the owner: two planned PRs, one unplanned, one card not merged. */
export function mainRelease(over: Partial<Release> = {}): Release {
  return release({
    id: "rel-18",
    name: "0.18.0",
    display_version: "0.18.0",
    items: [
      { item_id: "H-203", verdict: "pending", owner_note: null },
      { item_id: "H-247", verdict: "pending", owner_note: null },
      { item_id: "H-250", verdict: "pending", owner_note: null },
    ],
    tag: "",
    tag_name: "desktop-v0.18.0",
    commit: CUT,
    previous_commit: PREVIOUS,
    prs: [
      { number: 40, item_id: "H-203", merged_sha: CUT, title: "Card ids as links" },
      {
        number: 42,
        item_id: "H-247",
        merged_sha: "8f35e95a1b2c3d4e5f60718293a4b5c6d7e8f901",
        title: "Waiting for you on releases",
      },
    ],
    also_included: [
      {
        number: 43,
        item_id: "H-250",
        merged_sha: "f0678fc1a2b3c4d5e6f708192a3b4c5d6e7f8091",
        title: "Typo in the About box",
      },
    ],
    not_merged: ["H-259"],
    ...over,
  });
}

/** 0.17.5, tagged at the commit 0.18.0's range starts after. */
export function previousRelease(): Release {
  return mainRelease({
    id: "rel-17",
    name: "0.17.5",
    display_version: "0.17.5",
    status: "deployed",
    tag: "desktop-v0.17.5",
    commit: PREVIOUS,
    previous_commit: null,
  });
}

type Review = readonly [role: string, verdict: Verdict, stale?: boolean];

/** A merged PR's record, with its required roles and the reviews they gave. */
function prRecord(
  number: number,
  required: readonly string[],
  reviews: readonly Review[],
): PullRequest {
  return create(PullRequestSchema, {
    number,
    state: PrState.MERGED,
    requiredRoles: [...required],
    ownerReviewRequired: required.includes("owner"),
    reviews: reviews.map(([role, verdict, stale]) => ({
      role,
      verdict,
      stale: stale ?? false,
      sha: CUT,
    })),
  });
}

const OK = Verdict.APPROVED;

/** Every PR of 0.18.0 reviewed, as the canvas shows them. */
export const MAIN_RECORDS: ReadonlyMap<number, PullRequest> = new Map([
  [
    40,
    prRecord(
      40,
      ["owner", "architect", "ce", "ux"],
      [
        ["owner", OK],
        ["architect", OK],
        ["ce", OK],
        ["ux", OK],
      ],
    ),
  ],
  [
    42,
    prRecord(
      42,
      ["owner", "ce", "ux", "qa"],
      [
        ["owner", OK],
        ["ce", OK],
        ["ux", OK],
        ["qa", OK],
      ],
    ),
  ],
  [43, prRecord(43, ["architect"], [["architect", OK]])],
]);
