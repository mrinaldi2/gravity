// Needs-you rows for cleanups and disk space (H-275; H-261 §15.6, UX-051
// "needs"): a cleanup held 3 days or failed opens a choice, Remove anyway
// or Keep it; a computer under 20 GB free offers Clean up.

import type { AttentionRowJson } from "../../protocol/dashboard";
import type { RowViewProps } from "./NeedsYou";

export interface CleanupRowActions {
  /** Opens the choice for a held or failed cleanup. */
  readonly onDecideCleanup?: (row: AttentionRowJson) => void;
  /** Runs Clean up on the computer. */
  readonly onCleanUp?: (machine: string) => void;
  /** A Clean up is running on this computer. */
  readonly cleaningUp?: string | null;
}

/** The row for a cleanup or disk kind; undefined for any other kind. */
export function cleanupRow(r: AttentionRowJson, a: CleanupRowActions): RowViewProps | undefined {
  switch (r.kind) {
    case "cleanup_held": {
      const decide = a.onDecideCleanup;
      return {
        glyph: "⏸",
        tone: "you",
        title: r.title,
        meta: "Kept so nothing unsaved is lost. Remove it anyway, or keep it.",
        action: decide === undefined || r.cleanup_job_id === undefined ? undefined : "Decide…",
        label: `Decide: ${r.title}`,
        onAction: () => decide?.(r),
        verb: "Decide",
      };
    }
    case "disk_low": {
      const machine = r.machine ?? "";
      const busy = a.cleaningUp === machine;
      return {
        glyph: "◔",
        tone: "bad",
        title: r.title,
        meta: busy
          ? "Cleaning up…"
          : "Clean up removes merged and idle worktrees and old build output. Unsaved work is kept.",
        action: a.onCleanUp === undefined || machine.length === 0 ? undefined : "Clean up",
        label: `Clean up ${machine}`,
        onAction: () => a.onCleanUp?.(machine),
        disabled: busy,
        verb: "Clean up",
      };
    }
    default:
      return undefined;
  }
}
