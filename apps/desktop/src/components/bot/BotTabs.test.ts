import { describe, expect, it } from "vitest";
import { botTabs } from "./BotTabs";

describe("botTabs", () => {
  it("offers the browser where the daemon serves one", () => {
    expect(botTabs({ chat: true }, false)).toEqual(["chat", "terminal", "routines"]);
    expect(botTabs({ chat: true, browser: true }, false)).toEqual([
      "chat",
      "terminal",
      "browser",
      "routines",
    ]);
    expect(botTabs({ chat: false }, false)).toEqual(["terminal", "routines"]);
  });

  it("splits Chat (the owner thread) from Activity where threads exist (H-192)", () => {
    expect(botTabs({ chat: true, reports: true }, false)).toEqual([
      "reports",
      "chat",
      "activity",
      "terminal",
      "routines",
    ]);
    expect(botTabs({ chat: true, reports: true, peerTerminal: true }, true)).toEqual([
      "reports",
      "chat",
      "activity",
      "terminal",
    ]);
    // An older service: Chat is still the transcript, and no Activity.
    expect(botTabs({ chat: true }, false)).not.toContain("activity");
  });

  it("gives a linked bot its terminal when the daemon relays it", () => {
    expect(botTabs({ chat: true, browser: true }, true)).toEqual(["chat"]);
    expect(botTabs({ chat: true, browser: true, peerTerminal: true }, true)).toEqual([
      "chat",
      "terminal",
    ]);
    expect(botTabs({ chat: true, peerTerminal: true, peerBrowser: true }, true)).toEqual([
      "chat",
      "terminal",
      "browser",
    ]);
  });
});
