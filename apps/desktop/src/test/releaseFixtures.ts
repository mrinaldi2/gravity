// Release packages for tests and stories (H-018 §4A).

import type { Release, ReleaseDeployment } from "../protocol/releases";

const SHA = "a".repeat(64);

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
        sha256: SHA,
        built_at: "2026-10-05T10:00:00Z",
      },
    ],
    tests: [
      { machine: "mac", tester: "tester", build_sha256: SHA, result: "pass" },
      { machine: "win-pc", tester: "tester-win", build_sha256: SHA, result: "pass" },
    ],
    deployments: [],
    can_rule: true,
    rule_on: null,
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
