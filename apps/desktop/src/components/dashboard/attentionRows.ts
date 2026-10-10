// Needs-you rows for the kinds added with the projects home (H-128, UX-024
// §4): a question a bot asked the owner, a permission prompt, a bot waiting
// on the owner, work happening off the board. Each says what to do and opens
// where it is done.

import type { AttentionRowJson } from "../../protocol/dashboard";
import type { RowViewProps } from "./NeedsYou";
import type { PrRowActions } from "./prRows";
import { prRow } from "./prRows";

export interface AttentionActions extends PrRowActions {
  readonly onOpenBot?: (botId: string) => void;
  /** Opens a message to a bot quoting what it asked. */
  readonly onReply?: (botId: string, quote: string) => void;
  /** Opens Needs you, where permission prompts are answered. */
  readonly onOpenNeedsYou?: () => void;
  readonly onItem: (itemId: string, opener: HTMLElement) => void;
}

function botOf(r: AttentionRowJson): string | undefined {
  return r.bot?.bot_id === undefined || r.bot.bot_id.length === 0 ? undefined : r.bot.bot_id;
}

function question(r: AttentionRowJson, a: AttentionActions): RowViewProps {
  const who = r.bot?.name ?? "A bot";
  const item = r.item_id;
  const bot = botOf(r);
  if (item !== undefined && item.length > 0) {
    return {
      glyph: "?",
      tone: "you",
      title: r.title,
      meta: `A question for you on ${item}`,
      action: "Reply",
      label: `Reply on ${item}`,
      onAction: (event) => a.onItem(item, event.currentTarget),
      verb: "Reply",
    };
  }
  return {
    glyph: "?",
    tone: "you",
    title: r.title,
    meta: `${who} asks you`,
    action: bot === undefined || a.onReply === undefined ? undefined : "Reply",
    label: `Reply to ${who}`,
    onAction: () => {
      if (bot !== undefined) {
        a.onReply?.(bot, r.title);
      }
    },
    verb: "Reply",
  };
}

function openBot(
  r: AttentionRowJson,
  a: AttentionActions,
  glyph: string,
  meta: string,
): RowViewProps {
  const bot = botOf(r);
  const who = r.bot?.name ?? "the bot";
  return {
    glyph,
    title: r.title,
    meta,
    action: bot === undefined || a.onOpenBot === undefined ? undefined : "Open bot",
    label: `Open ${who}`,
    onAction: () => {
      if (bot !== undefined) {
        a.onOpenBot?.(bot);
      }
    },
    verb: "Open",
  };
}

/** The row for one of the newer kinds; undefined for one shown elsewhere. */
export function attentionRow(r: AttentionRowJson, a: AttentionActions): RowViewProps | undefined {
  switch (r.kind) {
    case "owner_question":
      return question(r, a);
    case "permission_prompt":
      return {
        glyph: "⚿",
        tone: "you",
        title: r.title,
        meta: "A bot asks permission to go on",
        action: a.onOpenNeedsYou === undefined ? undefined : "Answer",
        label: `Answer: ${r.title}`,
        onAction: () => a.onOpenNeedsYou?.(),
        verb: "Answer",
      };
    case "bot_waiting":
      return openBot(r, a, "⏸", "Waiting for you");
    case "off_board":
      return openBot(r, a, "↗", "Working on something not on the board");
    case "owner_action":
      // Listed under "Commands for you to run", with its Run button.
      return undefined;
    default:
      return prRow(r, a);
  }
}
