import { describe, expect, it, vi } from "vitest";
import type { AttentionRowJson } from "../../protocol/dashboard";
import { attentionRow } from "./attentionRows";
import type { AttentionActions } from "./attentionRows";

function actions(): AttentionActions & {
  readonly onReply: ReturnType<typeof vi.fn<(botId: string, quote: string) => void>>;
  readonly onOpenBot: ReturnType<typeof vi.fn<(botId: string) => void>>;
  readonly onOpenNeedsYou: ReturnType<typeof vi.fn<() => void>>;
  readonly onItem: ReturnType<typeof vi.fn<(itemId: string, opener: HTMLElement) => void>>;
} {
  return {
    onReply: vi.fn<(botId: string, quote: string) => void>(),
    onOpenBot: vi.fn<(botId: string) => void>(),
    onOpenNeedsYou: vi.fn<() => void>(),
    onItem: vi.fn<(itemId: string, opener: HTMLElement) => void>(),
  };
}

const BOT = { daemon_id: "mac", bot_id: "b2", name: "Desktop Dev" };
const click = { currentTarget: document.createElement("button") } as never;

describe("attentionRow", () => {
  it("replies to a question asked in a thread, quoting it", () => {
    const a = actions();
    const row: AttentionRowJson = {
      kind: "owner_question",
      id: "q1",
      title: "Should Resume also restart services?",
      bot: BOT,
    };
    const view = attentionRow(row, a);
    expect(view?.meta).toBe("Desktop Dev asks you");
    view?.onAction?.(click);
    expect(a.onReply).toHaveBeenCalledWith("b2", "Should Resume also restart services?");
  });

  it("opens the card a question was asked on", () => {
    const a = actions();
    const view = attentionRow(
      { kind: "owner_question", id: "q2", title: "Units?", item_id: "H-117", bot: BOT },
      a,
    );
    expect(view?.label).toBe("Reply on H-117");
    view?.onAction?.(click);
    expect(a.onItem).toHaveBeenCalledWith("H-117", expect.anything());
  });

  it("sends permission prompts to Needs you and waits to the bot", () => {
    const a = actions();
    attentionRow({ kind: "permission_prompt", id: "p", title: "Run cargo?" }, a)?.onAction?.(click);
    expect(a.onOpenNeedsYou).toHaveBeenCalled();
    attentionRow({ kind: "bot_waiting", id: "w", title: "Blocked", bot: BOT }, a)?.onAction?.(
      click,
    );
    expect(a.onOpenBot).toHaveBeenCalledWith("b2");
  });

  it("leaves owner actions to their own widget", () => {
    expect(
      attentionRow({ kind: "owner_action", id: "o", title: "Run" }, actions()),
    ).toBeUndefined();
  });
});
