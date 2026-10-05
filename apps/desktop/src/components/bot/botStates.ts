import type { Bot, BotState } from "../../protocol/entities";

/** The words for each bot state, shared with the iOS app (ux-glossary §2.1). */
export const BOT_STATE_LABEL: Readonly<Record<BotState, string>> = {
  starting: "Starting",
  ready: "Idle",
  working: "Working",
  waiting_for_user: "Waiting for you",
  waiting_for_approval: "Needs approval",
  rate_limited: "Rate limited",
  auth_failed: "Sign-in failed",
  crashed: "Crashed",
  stopping: "Stopping",
  stopped: "Stopped",
};

/**
 * The failure states whose reason the owner sees (ux-glossary rule 8).
 * "Waiting for you" names what is wanted: "Didn't connect — Restart bot".
 */
const REASON_SHOWN: ReadonlySet<BotState> = new Set<BotState>([
  "crashed",
  "waiting_for_user",
  "auth_failed",
  "rate_limited",
]);

/**
 * The state in words, an unknown one included. A failure state carries its
 * reason; every other reason is internal bookkeeping and stays hidden.
 */
export function botStateTitle(bot: Pick<Bot, "state" | "state_reason">): string {
  const label: string | undefined = BOT_STATE_LABEL[bot.state];
  const words = label ?? "Unknown";
  return REASON_SHOWN.has(bot.state) && bot.state_reason.length > 0
    ? `${words} — ${bot.state_reason}`
    : words;
}
