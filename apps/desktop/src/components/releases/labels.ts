// Owner-facing words for release packages (H-018 §4A.1). Every status and
// result carries a glyph and a word, so nothing depends on colour.

import type { Release, ReleaseStatus, ReleaseTest } from "../../protocol/releases";

export interface StatusLabel {
  readonly glyph: string;
  readonly word: string;
  /** Tone for the pill: what the owner should feel, never the only signal. */
  readonly tone: "wait" | "you" | "ok" | "bad" | "off";
}

const STATUS: Readonly<Record<ReleaseStatus, StatusLabel>> = {
  assembling: { glyph: "○", word: "Being packaged by DevOps", tone: "wait" },
  built: { glyph: "○", word: "Being tested", tone: "wait" },
  awaiting_owner: { glyph: "◐", word: "Ready for you to test", tone: "you" },
  held: { glyph: "⏸", word: "On hold", tone: "off" },
  repackaging: { glyph: "↻", word: "Approved in part: being repackaged", tone: "wait" },
  superseded: { glyph: "⤳", word: "Replaced by a newer package", tone: "off" },
  approved: { glyph: "●", word: "Approved: rolling out soon", tone: "ok" },
  deploying: { glyph: "◑", word: "Rolling out", tone: "ok" },
  paused: { glyph: "⏸", word: "Rollout paused", tone: "you" },
  partially_deployed: { glyph: "✗", word: "Rollout failed", tone: "bad" },
  deployed: { glyph: "✓", word: "Live on every computer", tone: "ok" },
  rejected: { glyph: "⊘", word: "Rejected", tone: "off" },
  rolled_back: { glyph: "↩", word: "Rolled back", tone: "off" },
  cancelled: { glyph: "⊘", word: "Cancelled by DevOps", tone: "off" },
};

export function statusLabel(status: ReleaseStatus): StatusLabel {
  return STATUS[status] ?? { glyph: "•", word: status, tone: "wait" };
}

/** What the release review shows as the package's name: its version. */
export function releaseTitle(release: Release): string {
  return release.display_version ?? release.name;
}

/** Packages still in play sit under Current; the rest are history. */
export function isCurrent(release: Release): boolean {
  return !["deployed", "rejected", "rolled_back", "superseded", "cancelled"].includes(
    release.status,
  );
}

export function testLabel(result: ReleaseTest["result"]): StatusLabel {
  switch (result) {
    case "pass":
      return { glyph: "✓", word: "Passed", tone: "ok" };
    case "fail":
      return { glyph: "✗", word: "Failed", tone: "bad" };
    default:
      return { glyph: "!", word: "Blocked", tone: "bad" };
  }
}

/** One machine's rollout row, from its deploy and rollback records. */
export function rolloutLabel(release: Release, machine: string): StatusLabel {
  const rows = release.deployments.filter((d) => d.machine === machine);
  const back = rows.find((d) => d.action === "rollback");
  const deploy = rows.find((d) => d.action === "deploy");
  if (back?.result === "rolled_back") {
    return { glyph: "↩", word: "Rolled back", tone: "off" };
  }
  if (back) {
    return { glyph: "◑", word: "Rolling back", tone: "wait" };
  }
  if (deploy?.result === "ok") {
    return { glyph: "✓", word: "Live", tone: "ok" };
  }
  if (deploy?.result === "failed") {
    return { glyph: "✗", word: "Failed", tone: "bad" };
  }
  if (deploy) {
    return { glyph: "◑", word: "Installing", tone: "wait" };
  }
  return { glyph: "○", word: "Queued", tone: "wait" };
}
