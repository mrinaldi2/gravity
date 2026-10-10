// Cleanups and disk space for the owner (H-275; H-261 §15.6): Remove anyway
// or Keep a held cleanup (the app's ticket or a paired device, on the
// board's home), each computer's disk report, and Clean up (the sweep plus
// build-cache trims). JSON requests on the app's own connection.

export interface DiskUse {
  readonly bot: { readonly id: string; readonly name: string };
  readonly workspace_bytes: number;
  readonly worktree_bytes: number;
  readonly cache_bytes: number;
  /** Old build output the sweep may remove. */
  readonly reclaimable_bytes: number;
}

export interface DiskReport {
  readonly machine: string;
  readonly free_bytes: number;
  readonly total_bytes: number;
  /** Biggest first. */
  readonly uses: readonly DiskUse[];
  readonly as_of: string;
}

interface CleanupItemJson {
  readonly job_id: string;
  readonly machine: string;
  readonly state: "queued" | "done" | "held" | "failed";
  readonly path: string;
  readonly reason: string;
  readonly freed_bytes: number;
  readonly salvaged: boolean;
}

export type CleanupRequestBody =
  | {
      readonly type: "cleanup_resolve";
      readonly project_id: string;
      readonly job_id: string;
      readonly action: "remove" | "keep";
    }
  /** Empty `machine`: this computer. */
  | { readonly type: "disk_report"; readonly machine?: string; readonly refresh?: boolean }
  | { readonly type: "cleanup_now"; readonly machine?: string };

export type CleanupReply =
  | { readonly type: "cleanup_item"; readonly cleanup_item: CleanupItemJson }
  | { readonly type: "disk_report"; readonly disk_report: DiskReport }
  | {
      readonly type: "cleanup_done";
      readonly machine: string;
      readonly trees?: number;
      readonly freed_bytes?: number;
      readonly disk_report?: DiskReport;
      /** A linked computer started it; its rows follow. */
      readonly started?: boolean;
    };

/** "14 GB", "300 MB": the size the owner reads. */
export function sizeText(bytes: number): string {
  if (bytes >= 1e9) {
    return `${(bytes / 1e9).toFixed(bytes >= 1e11 ? 0 : 1)} GB`;
  }
  if (bytes >= 1e6) {
    return `${Math.round(bytes / 1e6)} MB`;
  }
  return `${Math.round(bytes / 1e3)} KB`;
}
