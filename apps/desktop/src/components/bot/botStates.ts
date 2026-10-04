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

/** The state in words, an unknown one included, with the reason when there is one. */
export function botStateTitle(bot: Pick<Bot, "state" | "state_reason">): string {
  const label: string | undefined = BOT_STATE_LABEL[bot.state];
  const words = label ?? "Unknown";
  return bot.state_reason.length > 0 ? `${words} — ${bot.state_reason}` : words;
}
