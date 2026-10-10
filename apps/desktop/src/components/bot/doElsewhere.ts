import type { Bot } from "../../protocol/entities";

/**
 * Where the owner types into, or drives, a linked bot instead: its computer
 * takes no typing, permission answers or browser control from this one
 * (H-303). The daemon refuses in the same words. Null for a bot that runs
 * here.
 */
export function doElsewhere(bot: Bot): string | null {
  if (bot.peer == null) {
    return null;
  }
  return `Do it on ${bot.peer.name} or your phone: a linked computer can't type into or drive a bot there yet.`;
}
