// The words of a held or failed cleanup (UX-055): composed here from the
// row's fields, never the service's own reason text with its paths.

import type { AttentionRowJson, CleanupFields } from "../../protocol/dashboard";

function plural(n: number, one: string, many: string): string {
  return `${n} ${n === 1 ? one : many}`;
}

/** "Desktop Dev's worktree on imac", or "A worktree on imac". */
export function whose(f: CleanupFields, start = true): string {
  const machine = f.machine ?? "a computer";
  const name = f.bot?.name;
  if (name !== undefined && name.length > 0) {
    return `${name}'s worktree on ${machine}`;
  }
  return `${start ? "A" : "a"} worktree on ${machine}`;
}

/** "1 uncommitted file and 1 unpushed commit", or "" with none. */
export function unsaved(f: CleanupFields): string {
  const parts: string[] = [];
  if ((f.uncommitted ?? 0) > 0) {
    parts.push(plural(f.uncommitted ?? 0, "uncommitted file", "uncommitted files"));
  }
  if ((f.unpushed ?? 0) > 0) {
    parts.push(plural(f.unpushed ?? 0, "unpushed commit", "unpushed commits"));
  }
  return parts.join(" and ");
}

/** "held 3 days", from when it was held. */
function heldFor(since: string | undefined, now: number): string | null {
  if (since === undefined) {
    return null;
  }
  const days = Math.floor((now - Date.parse(since)) / 86_400_000);
  if (Number.isNaN(days)) {
    return null;
  }
  return days >= 1 ? `held ${plural(days, "day", "days")}` : "held today";
}

function failed(f: CleanupFields): boolean {
  return f.state === "failed";
}

/** The row's title and meta; the daemon's title for an older service. */
export function cleanupWords(
  r: AttentionRowJson,
  now = Date.now(),
): { readonly title: string; readonly meta: string } {
  const f = r.cleanup;
  if (f === undefined) {
    return {
      title: r.title,
      meta: "Kept so nothing unsaved is lost. Remove it anyway, or keep it.",
    };
  }
  if (failed(f)) {
    return {
      title: `Couldn't remove ${whose(f, false)}: ${f.reason ?? "it failed"}`,
      meta: "It's tried again in the next daily cleanup. Keep it to stop asking.",
    };
  }
  const what = unsaved(f) || (f.reason ?? "");
  const meta = [
    f.pr_number ? `#${f.pr_number}` : null,
    f.salvaged ? "its changes are saved in the salvage folder" : null,
    heldFor(f.since, now),
  ].filter((p): p is string => p !== null);
  return { title: `${whose(f)} is kept: ${what}`, meta: meta.join(" · ") };
}
