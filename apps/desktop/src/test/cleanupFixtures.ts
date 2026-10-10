// Cleanup and disk rows and a disk report for tests and stories (H-275).

import type { DiskReport } from "../protocol/cleanup";
import type { AttentionRowJson } from "../protocol/dashboard";

/** Held 3 days on imac, its one change salvaged. */
export const HELD_ROW: AttentionRowJson = {
  kind: "cleanup_held",
  id: "cleanup_held:mac:job-1",
  title: "Desktop Dev's worktree on imac is kept: 1 uncommitted file",
  cleanup_job_id: "job-1",
  cleanup: {
    state: "held",
    machine: "imac",
    bot: { bot_id: "b1", name: "Desktop Dev" },
    pr_number: 42,
    uncommitted: 1,
    salvaged: true,
    since: new Date(Date.now() - 3 * 86_400_000 - 60_000).toISOString(),
  },
};

/** Failed on win-pc: nothing was saved, so only Keep it is offered. */
export const FAILED_ROW: AttentionRowJson = {
  kind: "cleanup_held",
  id: "cleanup_held:mac:job-2",
  title: "Couldn't remove iOS Dev's worktree on win-pc: the folder is in use",
  cleanup_job_id: "job-2",
  cleanup: {
    state: "failed",
    machine: "win-pc",
    bot: { bot_id: "b2", name: "iOS Dev" },
    salvaged: false,
    reason: "the folder is in use",
    since: new Date(Date.now() - 86_400_000).toISOString(),
  },
};

export const LOW_ROW: AttentionRowJson = {
  kind: "disk_low",
  id: "disk_low:mac:mac",
  title: "mac is low on disk: 14.0 GB free; 9.1 GB is old build output",
  machine: "mac",
};

const use = (id: string, name: string, ws: number, trees: number, cache: number, old: number) => ({
  bot: { id, name },
  workspace_bytes: ws,
  worktree_bytes: trees,
  cache_bytes: cache,
  reclaimable_bytes: old,
});

export const DISK_REPORT: DiskReport = {
  machine: "mac",
  free_bytes: 14_000_000_000,
  total_bytes: 494_000_000_000,
  uses: [
    use("b1", "Desktop Dev", 400_000_000, 6_200_000_000, 21_000_000_000, 0),
    use("b2", "iOS Dev", 900_000_000, 3_100_000_000, 9_100_000_000, 9_100_000_000),
    use("b3", "Architect", 120_000_000, 0, 0, 0),
    use("b4", "Team Lead", 80_000_000, 0, 0, 0),
    use("b5", "UX Designer", 30_000_000, 0, 0, 0),
  ],
  as_of: "2026-10-10T09:00:00Z",
};
