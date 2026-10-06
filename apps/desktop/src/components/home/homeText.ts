// Owner-facing words for the projects home (UX-024 §3). Every count and state
// is spelled out, so a card reads the same without colour.

import { timestampDate } from "@bufbuild/protobuf/wkt";
import type { Timestamp } from "@bufbuild/protobuf/wkt";
import {
  Source_State,
  type AttentionSummary,
  type ProjectRow,
  type ReleaseBrief,
  type Source,
} from "../../protocol/gen/hermes/home/v1/home_pb";
import type { ReleaseStatus } from "../../protocol/releases";
import { age } from "../control/decisions";
import { statusLabel } from "../releases/labels";

/** Singular and plural words per attention kind, in needs-you points order. */
const KIND_WORDS: readonly (readonly [string, string, string])[] = [
  ["release_awaiting", "release to test", "releases to test"],
  ["decision", "decision", "decisions"],
  ["owner_action", "Run card", "Run cards"],
  ["p0_item", "P0 item", "P0 items"],
  ["permission_prompt", "approval", "approvals"],
  ["owner_question", "question for you", "questions for you"],
  ["relayed_rulings", "ruling to confirm", "rulings to confirm"],
  ["bot_waiting", "bot waiting for you", "bots waiting for you"],
  ["off_board", "task off the board", "tasks off the board"],
  ["serving_off", "computer not serving", "computers not serving"],
];

/** "1 release to test · 2 decisions · 1 Run card": the top three reasons. */
export function whyLine(attention: AttentionSummary | undefined): string {
  if (attention === undefined || attention.count === 0) {
    return "Nothing needs you";
  }
  const parts = KIND_WORDS.flatMap(([kind, one, many]) => {
    const n = attention.byKind[kind] ?? 0;
    return n > 0 ? [`${n} ${n === 1 ? one : many}`] : [];
  });
  return parts.length > 0 ? parts.slice(0, 3).join(" · ") : plural(attention.count, "item");
}

function plural(n: number, word: string): string {
  return `${n} ${word}${n === 1 ? "" : "s"}`;
}

export interface ReleasePill {
  readonly text: string;
  readonly tone: "wait" | "you" | "ok" | "bad" | "off";
}

/** "◐ 0.17.0 ready for you to test"; "○ No release yet" without one. */
export function releasePill(brief: ReleaseBrief | undefined): ReleasePill {
  if (brief === undefined || brief.version.length === 0) {
    return { text: "○ No release yet", tone: "off" };
  }
  const label = statusLabel(brief.state as ReleaseStatus);
  const word = label.word.charAt(0).toLowerCase() + label.word.slice(1);
  return { text: `${label.glyph} ${brief.version} ${word}`, tone: label.tone };
}

/** "5 tasks". */
export function runningLine(row: ProjectRow): string {
  return plural(row.openTasks, "task");
}

/** "8 · 5 working", or "3 · all idle". */
export function botsLine(row: ProjectRow): string {
  if (row.bots === 0) {
    return "No bots yet";
  }
  return row.botsWorking > 0
    ? `${row.bots} · ${row.botsWorking} working`
    : `${row.bots} · all idle`;
}

function iso(at: Timestamp | undefined): string | undefined {
  return at === undefined ? undefined : timestampDate(at).toISOString();
}

/** "09:05" today, else "12 Sep 09:05". */
export function clock(at: Timestamp, now: number): string {
  const date = timestampDate(at);
  const time = date.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  return new Date(now).toDateString() === date.toDateString()
    ? time
    : `${date.toLocaleDateString([], { day: "numeric", month: "short" })} ${time}`;
}

/** "Last activity 3 days ago", or nothing when the service never saw any. */
export function lastActivity(row: ProjectRow, now: number): string | undefined {
  const at = iso(row.lastActivityAt);
  return at === undefined ? undefined : `Last activity ${age(at, now)}`;
}

/**
 * Why a card shows less than it should: one note per computer answering from
 * old data or not at all, e.g. "imac is offline · last seen 10:20".
 */
export function staleNotes(row: ProjectRow, sources: readonly Source[], now: number): string[] {
  return row.staleSources.map((daemonId) => {
    const source = sources.find((item) => item.daemonId === daemonId);
    const name =
      source?.name ??
      row.members.find((member) => member.daemonId === daemonId)?.computerName ??
      "A computer";
    const seen = source?.asOf === undefined ? "never seen" : `last seen ${clock(source.asOf, now)}`;
    switch (source?.state) {
      case Source_State.TIMEOUT:
        return `${name} didn't answer in time · ${seen}`;
      case Source_State.OLD_VERSION:
        return `${name} needs an update to show everything`;
      default:
        return `${name} is offline · ${seen}`;
    }
  });
}

/** "3 projects · 2 computers". */
export function homeSummary(projects: number, computers: number): string {
  return `${plural(projects, "project")} · ${plural(Math.max(1, computers), "computer")}`;
}

/** The ranking rule, as the "Why this order?" tooltip states it. */
export const ORDER_RULE =
  "Projects are ranked by how much they need you: releases to test and urgent decisions count most, then Run cards and P0 items, then other decisions and approvals, then questions and rulings to confirm. Each one counts more the longer it waits. Pinned projects stay on top; projects that need nothing sort by recent activity.";
