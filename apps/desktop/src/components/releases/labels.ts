// Owner-facing words for release packages (H-018 §4A.1). Every status and
// result carries a glyph and a word, so nothing depends on colour.

import type { Release, ReleaseEvent, ReleaseStatus, ReleaseTest } from "../../protocol/releases";

export interface StatusLabel {
  readonly glyph: string;
  readonly word: string;
  /** Tone for the pill: what the owner should feel, never the only signal. */
  readonly tone: "wait" | "you" | "ok" | "bad" | "off";
}

const STATUS: Readonly<Record<ReleaseStatus, StatusLabel>> = {
  planned: { glyph: "◌", word: "Planned: work in progress", tone: "wait" },
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
};

export function statusLabel(status: ReleaseStatus): StatusLabel {
  return STATUS[status] ?? { glyph: "•", word: status, tone: "wait" };
}

/** "1 item", "2 items". */
export function plural(n: number, word: string): string {
  return `${n} ${word}${n === 1 ? "" : "s"}`;
}

/** What the release review shows as the package's name: its version. */
export function releaseTitle(release: Release): string {
  return release.display_version ?? release.name;
}

/** Packages still in play sit under Current; the rest are history. */
export function isCurrent(release: Release): boolean {
  return !["deployed", "rejected", "rolled_back", "superseded"].includes(release.status);
}

/** A bot's name by id; undefined when this device doesn't know the bot. */
export type BotName = (id: string) => string | undefined;

type EventText = (event: ReleaseEvent, who: string) => string;

/** A line per event kind; the note is a bot's own words, so it is quoted. */
const EVENT_LINES: Readonly<Record<string, EventText>> = {
  cancelled: (event, who) => {
    const what = `${who} cancelled ${event.release_name}, the package that was going to replace this one.`;
    return event.note ? `${what} Their note: “${event.note}”.` : what;
  },
  // H-121: closed because a later, deployed release contains it.
  deployed_via: (event, who) =>
    `${who} closed ${event.release_name}: ${event.note ?? "deployed through a later release"}.`,
  lead_ticked: (event, who) => {
    const d = event.detail ?? {};
    const verb = d.passed === false ? "marked failed" : "ticked";
    const what = `${who} ${verb} “${d.text ?? "a criterion"}” on ${d.item_id ?? "an item"} on the lead's own evidence.`;
    return event.note ? `${what} Evidence: “${event.note}”.` : what;
  },
};

/** One line for an event on a package, e.g. a successor DevOps cancelled. */
export function eventLine(event: ReleaseEvent, botName: BotName): string {
  const who =
    event.actor === "owner" || event.actor.startsWith("device:")
      ? "You"
      : (botName(event.actor) ?? "A bot");
  const special = EVENT_LINES[event.kind]?.(event, who) ?? scopeLine(event, who);
  if (special) {
    return special;
  }
  const what = `${who}: ${event.kind} ${event.release_name}`;
  return event.note ? `${what}: ${event.note}` : what;
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

/**
 * Every computer the package goes to. A package reaches the owner only once
 * each required computer has a result, so its tests name them all, including
 * those the rollout hasn't reached yet (§4A.4).
 */
export function targetMachines(release: Release): string[] {
  const tested = release.tests.map((t) => t.machine);
  return [...new Set([...tested, ...release.deployments.map((d) => d.machine)])];
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
  return release.status === "paused"
    ? { glyph: "○", word: "Not started", tone: "off" }
    : { glyph: "○", word: "Queued", tone: "wait" };
}

/** The planning events of a package (H-137): planned, scope changed, assembled. */
function scopeLine(event: ReleaseEvent, who: string): string | null {
  const d = event.detail ?? {};
  if (event.kind === "planned") {
    return `${who} planned ${event.release_name} with ${(d.items ?? []).join(", ")}.`;
  }
  if (event.kind === "assembled") {
    return `${who} started packaging ${event.release_name}: every item reached Verify.`;
  }
  if (event.kind !== "items_changed") {
    return null;
  }
  const parts = [
    d.added?.length ? `added ${d.added.join(", ")}` : "",
    d.removed?.length ? `took out ${d.removed.join(", ")}` : "",
  ].filter(Boolean);
  const what = `${who} ${parts.join(" and ")} in ${event.release_name}.`;
  return event.note ? `${what} Why: “${event.note}”.` : what;
}
