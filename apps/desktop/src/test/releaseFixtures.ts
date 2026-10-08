// Release packages for tests and stories (H-018 §4A).

import type { Release, ReleaseDeployment, ReleaseEvent } from "../protocol/releases";

/** A successor DevOps cancelled before submitting it (ARCH-R25). */
export const CANCELLED: ReleaseEvent = {
  release_id: "rel-2",
  release_name: "0.16.1",
  related_id: "rel-1",
  kind: "cancelled",
  actor: "ops",
  note: "took H-021 along",
  at: "2026-10-05T14:00:00Z",
};

/** The mac build and the Windows build: each computer tests its own. */
export const MAC_SHA = "a".repeat(64);
export const WIN_SHA = "b".repeat(64);
/** The commit both builds were made from. */
const SOURCE_COMMIT = "1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9a0b";

/** A package of two items waiting for the owner, tested on mac and win-pc. */
export function release(over: Partial<Release> = {}): Release {
  return {
    id: "rel-1",
    project_id: "p1",
    name: "R-2026-W41",
    display_version: "0.16.0",
    status: "awaiting_owner",
    decision_id: "dec-rel-1",
    supersedes: null,
    install_mode: "side_by_side",
    rollback_to: null,
    changelog: "New: release packages you approve.\nFixed: the board loads on linked projects.",
    how_to_test: [
      {
        item_id: "H-017",
        platform: "desktop-mac",
        steps: ["Open the Releases tab", "Approve the package"],
      },
    ],
    frozen_at: "2026-10-05T10:20:00Z",
    frozen_hash: "f".repeat(64),
    created_by: "ops",
    version: 4,
    paused_reason: null,
    held_note: null,
    remind_at: null,
    items: [
      { item_id: "H-017", verdict: "pending", owner_note: null },
      { item_id: "H-020", verdict: "pending", owner_note: null },
    ],
    builds: [
      {
        platform: "desktop-mac",
        version: "0.16.0",
        artifact: "/builds/The Hermes 0.16.0.dmg",
        url: null,
        install_url: null,
        sha256: MAC_SHA,
        built_at: "2026-10-05T10:00:00Z",
        source_commit: SOURCE_COMMIT,
      },
      {
        platform: "desktop-win",
        version: "0.16.0",
        artifact: "/builds/The Hermes 0.16.0.msi",
        url: null,
        install_url: null,
        sha256: WIN_SHA,
        built_at: "2026-10-05T10:05:00Z",
        source_commit: SOURCE_COMMIT,
      },
    ],
    tests: [
      { machine: "mac", tester: "tester", build_sha256: MAC_SHA, result: "pass" },
      { machine: "win-pc", tester: "tester-win", build_sha256: WIN_SHA, result: "pass" },
    ],
    deployments: [],
    events: [],
    can_rule: true,
    rule_on: null,
    // A current service's view (H-247): the ruling waits on the owner while
    // the package is awaiting them. Pass `owner_blockers` to say otherwise.
    owner_blockers:
      (over.status ?? "awaiting_owner") === "awaiting_owner"
        ? [
            {
              kind: "ruling",
              id: "dec-rel-1",
              title: over.display_version ?? "0.16.0",
              item_id: null,
              bot: null,
              computer: null,
              created_at: "2026-10-05T09:00:00Z",
            },
          ]
        : [],
    ...over,
  };
}

export function deployment(over: Partial<ReleaseDeployment> = {}): ReleaseDeployment {
  return {
    machine: "mac",
    action: "deploy",
    executor: "tester",
    task_id: "task-1",
    result: null,
    smoke: null,
    log_artifact: null,
    started_at: "2026-10-05T16:02:00Z",
    at: null,
    ...over,
  };
}

export const RELEASE_TITLES: ReadonlyMap<string, string> = new Map([
  ["H-017", "Release packages and the deploy gate"],
  ["H-020", "Board home on linked computers"],
]);
