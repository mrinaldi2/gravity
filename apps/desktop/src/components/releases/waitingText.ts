// The words of "Waiting for you" and the Progress "Now:" line (UX-048 §2,
// §3): each owner blocker as a row, and what a release waits on now, always
// naming who: you, DevOps, a computer or the cards still in progress.

import type { OwnerBlocker, Release } from "../../protocol/releases";
import type { BotName } from "./labels";

const MINUTE_MS = 60_000;

/** "25m", "1h", "3d": how long it has waited, as a row's meta says it. */
export function shortAge(iso: string, now: number): string {
  const then = Date.parse(iso);
  if (!Number.isFinite(then)) {
    return "";
  }
  const minutes = Math.max(0, Math.floor((now - then) / MINUTE_MS));
  if (minutes < 60) {
    return `${Math.max(1, minutes)}m`;
  }
  const hours = Math.floor(minutes / 60);
  return hours < 24 ? `${hours}h` : `${Math.floor(hours / 24)}d`;
}

/** A row's glyph and what it asks, without who, where or when. */
export function blockerWords(b: OwnerBlocker, botName: BotName): { glyph: string; what: string } {
  const bot = (b.bot && botName(b.bot)) ?? "A bot";
  switch (b.kind) {
    case "ruling":
      return { glyph: "◐", what: `Test ${b.title} and rule on it` };
    case "run":
      // UX-049 §1: the Run card's own title; the reason stays on the card.
      return { glyph: "▶", what: `${bot} asks you to run a command on ${computerOf(b)}` };
    case "decision":
      return { glyph: "◆", what: `Decide: ${b.title}` };
    case "question":
      return { glyph: "?", what: `${bot} asks: “${b.title}”` };
    case "permission":
      return { glyph: "⚑", what: `${bot} wants to run ${b.title}` };
    default:
      return { glyph: "•", what: b.title };
  }
}

/** The row's button (UX-048 §2). */
export function blockerAction(b: OwnerBlocker): string {
  switch (b.kind) {
    case "ruling":
      return "Review…";
    case "run":
      return "Run…";
    case "permission":
      return "Review";
    default:
      return "Answer";
  }
}

/** Who to name in a row's meta: the bot, where the row's title doesn't. */
export function blockerBy(b: OwnerBlocker, botName: BotName): string | null {
  if (b.kind !== "decision" || !b.bot) {
    return null;
  }
  return botName(b.bot) ?? "a bot";
}

function computerOf(b: OwnerBlocker): string {
  return b.computer ?? "a computer";
}

/** What the owner is waited on for, after "waiting for you" (UX-049 §2). */
function waitedFor(b: OwnerBlocker, botName: BotName): string {
  const bot = (b.bot && botName(b.bot)) ?? "a bot";
  const on = b.item_id ? ` (${b.item_id})` : "";
  switch (b.kind) {
    case "run":
      return `to run a command on ${computerOf(b)}${on}`;
    case "decision":
      return `to decide “${b.title}”`;
    case "question":
      return `to answer ${bot}${on}`;
    case "permission":
      return `to review ${bot}'s request to run ${b.title}`;
    default:
      return `to test ${b.title} and rule on it`;
  }
}

function andCount(items: readonly string[], named: number): string {
  const shown = items.slice(0, named).join(", ");
  return items.length > named ? `${shown}, +${items.length - named}` : shown;
}

/** Releases that are over: no "Now:" for them. */
const OVER = new Set(["superseded", "deployed", "rejected", "rolled_back"]);

export function isUnderWay(release: Release): boolean {
  return !OVER.has(release.status);
}

/**
 * The first line of Progress (UX-048 §3), or null for a release that is
 * over or a service that doesn't say what waits for the owner.
 */
export function nowLine(release: Release, botName: BotName): string | null {
  if (!isUnderWay(release) || release.owner_blockers === undefined) {
    return null;
  }
  const first = release.owner_blockers[0];
  if (first) {
    return `Now: waiting for you ${waitedFor(first, botName)}.`;
  }
  return (
    inProgress(release) ??
    building(release) ??
    testing(release) ??
    assembling(release) ??
    rollingOut(release) ??
    "Now: nothing is blocking it."
  );
}

/** Being put together, with nothing more particular to name (UX-050 §2). */
function assembling(release: Release): string | null {
  return ["assembling", "built"].includes(release.status)
    ? "Now: DevOps is assembling the package."
    : null;
}

/** Cards still being worked on, while the release is planned or assembling. */
function inProgress(release: Release): string | null {
  const busy = (release.plan ?? []).filter((p) => !p.ready);
  if (busy.length === 0 || !["planned", "assembling"].includes(release.status)) {
    return null;
  }
  const where = busy.map((p) => `${p.item_id} in ${p.column_name ?? p.column_key}`);
  const items = busy.length === 1 ? "1 item" : `${busy.length} items`;
  return `Now: ${items} still in progress (${andCount(where, 2)}).`;
}

function building(release: Release): string | null {
  const early = ["planned", "assembling"].includes(release.status);
  return early && release.builds.length === 0 ? "Now: DevOps is building the packages." : null;
}

function testing(release: Release): string | null {
  const required = release.readiness?.tests_required ?? [];
  const passed = release.readiness?.tests_passed ?? [];
  const left = required.filter((m) => !passed.includes(m));
  if (left.length === 0 || !["assembling", "built"].includes(release.status)) {
    return null;
  }
  return `Now: testing on ${left[0]} (${passed.length} of ${required.length} computers).`;
}

function rollingOut(release: Release): string | null {
  if (!["approved", "deploying", "partially_deployed", "paused"].includes(release.status)) {
    return null;
  }
  // Never "0 of 0" (UX-050 §2).
  if (release.status === "approved" && release.deployments.length === 0) {
    return "Now: approved, the rollout starts soon.";
  }
  const done = release.deployments.filter((d) => d.action === "deploy" && d.result === "ok");
  const of = (release.deploys_to ?? []).length || release.deployments.length;
  return of === 0
    ? "Now: rolling out."
    : `Now: rolling out, ${done.length} of ${of} computers updated.`;
}
