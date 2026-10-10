// Checks and cleanup in words (UX-051 decision 6, H-261 §15.6): a check is
// always "at <commit>", with the computer it ran on; cleanup says what was
// freed, or where it is held.

import { timestampDate } from "@bufbuild/protobuf/wkt";
import type {
  CheckRun,
  Cleanup,
  CleanupItem,
  PullRequest,
} from "../../protocol/gen/hermes/pr/v1/pr_pb";
import { CheckResult, CleanupKind, CleanupState } from "../../protocol/gen/hermes/pr/v1/pr_pb";
import type { Tone } from "./prText";

const RESULTS: Readonly<
  Record<CheckResult, { readonly glyph: string; readonly word: string; readonly tone: Tone }>
> = {
  [CheckResult.UNSPECIFIED]: { glyph: "○", word: "not started", tone: "dim" },
  [CheckResult.QUEUED]: { glyph: "◌", word: "queued", tone: "dim" },
  [CheckResult.RUNNING]: { glyph: "◌", word: "running", tone: "dim" },
  [CheckResult.PASS]: { glyph: "✓", word: "passed", tone: "ok" },
  [CheckResult.FAIL]: { glyph: "✕", word: "failed", tone: "bad" },
  [CheckResult.ERROR]: { glyph: "⚠", word: "couldn't run", tone: "warn" },
};

export function checkResult(check: CheckRun): {
  readonly glyph: string;
  readonly word: string;
  readonly tone: Tone;
} {
  return RESULTS[check.result];
}

/** "mac · 14m", "win-pc", "waiting for a computer". */
export function checkWhere(check: CheckRun): string {
  const parts = [check.ranOn || "waiting for a computer"];
  if (check.startedAt !== undefined && check.finishedAt !== undefined) {
    const seconds = Math.round(
      (timestampDate(check.finishedAt).getTime() - timestampDate(check.startedAt).getTime()) / 1000,
    );
    parts.push(seconds < 60 ? `${seconds}s` : `${Math.round(seconds / 60)}m`);
  }
  return parts.join(" · ");
}

/** The PR's checks on its latest commit, and those on earlier commits (folded away). */
export function splitChecks(pr: PullRequest): {
  readonly head: CheckRun[];
  readonly earlier: CheckRun[];
} {
  return {
    head: pr.checks.filter((c) => c.sha === pr.headSha),
    earlier: pr.checks.filter((c) => c.sha !== pr.headSha),
  };
}

function plural(n: number, one: string, many: string): string {
  return `${n} ${n === 1 ? one : many}`;
}

/** One line for a set of checks: failures first, then couldn't run, then running, then passed. */
export function checksSummary(checks: readonly CheckRun[]): {
  readonly text: string;
  readonly tone: Tone;
} {
  const count = (result: CheckResult): number => checks.filter((c) => c.result === result).length;
  const failed = count(CheckResult.FAIL);
  const errored = count(CheckResult.ERROR);
  const pending =
    count(CheckResult.RUNNING) + count(CheckResult.QUEUED) + count(CheckResult.UNSPECIFIED);
  if (checks.length === 0) {
    return { text: "○ No checks yet", tone: "dim" };
  }
  if (failed > 0) {
    return { text: `✕ ${plural(failed, "check", "checks")} failed`, tone: "bad" };
  }
  if (errored > 0) {
    return { text: `⚠ ${plural(errored, "check", "checks")} couldn't run`, tone: "warn" };
  }
  if (pending > 0) {
    return { text: "◌ Checks running", tone: "dim" };
  }
  return { text: `✓ ${plural(checks.length, "check", "checks")}`, tone: "ok" };
}

function gigabytes(bytes: bigint): string {
  return `${(Number(bytes) / 1e9).toFixed(1)} GB`;
}

const KIND_WORDS: Readonly<Record<CleanupKind, string>> = {
  [CleanupKind.UNSPECIFIED]: "item",
  [CleanupKind.WORKTREE]: "worktree",
  [CleanupKind.BUILD_OUTPUT]: "build output",
  [CleanupKind.BUILD_CACHE]: "build cache",
  [CleanupKind.REMOTE_BRANCH]: "branch",
  [CleanupKind.CHECK_WORKTREE]: "check worktree",
  [CleanupKind.DOCS_PREVIEW]: "docs preview",
};

const STATE_WORDS: Readonly<Record<CleanupState, string>> = {
  [CleanupState.UNSPECIFIED]: "",
  [CleanupState.QUEUED]: "◌ queued",
  [CleanupState.RUNNING]: "◌ running",
  [CleanupState.DONE]: "✓ removed",
  [CleanupState.HELD]: "⏸ held",
  [CleanupState.FAILED]: "✕ failed",
};

/** One cleanup item: "imac · worktree · /path · ⏸ held: Uncommitted changes (salvaged)". */
export function cleanupItemLine(item: CleanupItem): string {
  const state = STATE_WORDS[item.state];
  const why = item.reason ? `: ${item.reason}` : "";
  const freed = item.freedBytes > 0n ? ` · ${gigabytes(item.freedBytes)} freed` : "";
  return `${item.machine} · ${KIND_WORDS[item.kind]} · ${item.path} · ${state}${why}${freed}`;
}

/** The PR's cleanup line (H-261 §15.6), or null when there is nothing to clean yet. */
export function cleanupLine(
  cleanup: Cleanup | undefined,
): { readonly text: string; readonly tone: Tone } | null {
  if (cleanup === undefined) {
    return null;
  }
  const stuck = cleanup.items.find((i) => i.state === cleanup.state);
  switch (cleanup.state) {
    case CleanupState.QUEUED:
    case CleanupState.RUNNING:
      return { text: "◌ Cleaning up…", tone: "dim" };
    case CleanupState.HELD:
      return {
        text: `⏸ Cleanup held on ${stuck?.machine ?? "a computer"}: ${stuck?.reason ?? "held"}`,
        tone: "warn",
      };
    case CleanupState.FAILED:
      return {
        text: `✕ Cleanup failed on ${stuck?.machine ?? "a computer"}: ${stuck?.reason ?? "failed"}`,
        tone: "bad",
      };
    case CleanupState.DONE: {
      const trees = cleanup.items.filter((i) => i.kind === CleanupKind.WORKTREE);
      const machines = [...new Set(trees.map((i) => i.machine))].join(", ");
      const parts = ["Cleaned up ✓"];
      if (trees.length > 0) {
        parts.push(
          `${trees.length === 1 ? "1 worktree" : `${trees.length} worktrees`} on ${machines}`,
        );
      }
      parts.push(`${gigabytes(cleanup.freedBytes)} freed`);
      if (cleanup.branchDeleted) {
        parts.push("branch deleted");
      }
      return { text: parts.join(" · "), tone: "ok" };
    }
    default:
      return null;
  }
}
