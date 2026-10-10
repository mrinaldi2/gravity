// Cleanup and disk rows and a disk report for tests and stories (H-275).

import type { DiskReport } from "../protocol/cleanup";
import type { AttentionRowJson } from "../protocol/dashboard";

export const HELD_ROW: AttentionRowJson = {
  kind: "cleanup_held",
  id: "cleanup_held:mac:job-1",
  title:
    "Cleanup held on imac: gravity-wt-desktopdev-a: 1 uncommitted path(s) (salvaged to salvage/p1/42/gravity-wt-desktopdev-a)",
  cleanup_job_id: "job-1",
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
