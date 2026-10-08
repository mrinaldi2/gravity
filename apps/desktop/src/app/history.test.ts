import { afterEach, describe, expect, it, vi } from "vitest";
import {
  HISTORY_LIMIT,
  captureScroll,
  currentEntry,
  pushHistory,
  restoreScroll,
  startHistory,
  stepHistory,
  updateCurrent,
} from "./history";
import type { Selection } from "./selection";

const board: Selection = { kind: "project", projectId: "p1", tab: "board" };
const bot: Selection = { kind: "bot", botId: "b1", tab: "activity" };

afterEach(() => {
  vi.unstubAllGlobals();
  document.body.innerHTML = "";
});

describe("app history", () => {
  it("goes back and forward, and a new view drops what was ahead", () => {
    let h = pushHistory(pushHistory(startHistory({ kind: "home" }), bot), board);
    h = stepHistory(h, -1) ?? h;
    expect(currentEntry(h).selection).toEqual(bot);
    expect(currentEntry(stepHistory(h, 1) ?? h).selection).toEqual(board);
    h = pushHistory(h, { kind: "control" });
    expect(stepHistory(h, 1)).toBeNull();
    expect(h.entries.map((e) => e.selection.kind)).toEqual(["home", "bot", "control"]);
  });

  it("keeps the last 50 views", () => {
    let h = startHistory({ kind: "home" });
    for (let i = 0; i < 60; i += 1) {
      h = pushHistory(h, { kind: "bot", botId: `b${i}` });
    }
    expect(h.entries).toHaveLength(HISTORY_LIMIT);
    expect(currentEntry(h).selection).toEqual({ kind: "bot", botId: "b59" });
  });

  it("records a tab moved to inside the view", () => {
    const h = updateCurrent(pushHistory(startHistory({ kind: "home" }), bot), {
      selection: { kind: "bot", botId: "b1", tab: "chat" },
    });
    expect(currentEntry(h).selection).toEqual({ kind: "bot", botId: "b1", tab: "chat" });
  });

  it("puts a pane back where it was once its content is tall enough", () => {
    const frames: FrameRequestCallback[] = [];
    vi.stubGlobal("requestAnimationFrame", (cb: FrameRequestCallback) => frames.push(cb));
    vi.stubGlobal("cancelAnimationFrame", () => undefined);
    document.body.innerHTML = '<main class="main"><div><section></section></div></main>';
    const root = document.querySelector("main") as HTMLElement;
    const pane = root.querySelector("section") as HTMLElement;
    let tall = 0;
    Object.defineProperty(pane, "scrollHeight", { get: () => tall });
    Object.defineProperty(pane, "clientHeight", { get: () => 100 });
    pane.scrollTop = 300;
    const marks = captureScroll(root);
    expect(marks).toEqual([{ path: [0, 0], top: 300 }]);
    pane.scrollTop = 0;

    const t = 0;
    restoreScroll(
      () => root,
      marks,
      () => t,
    );
    frames.shift()?.(0);
    expect(pane.scrollTop).toBe(0);
    tall = 500;
    frames.shift()?.(0);
    expect(pane.scrollTop).toBe(300);
    expect(frames).toHaveLength(0);
  });

  it("goes as far as it can when the content never grows", () => {
    const frames: FrameRequestCallback[] = [];
    vi.stubGlobal("requestAnimationFrame", (cb: FrameRequestCallback) => frames.push(cb));
    vi.stubGlobal("cancelAnimationFrame", () => undefined);
    document.body.innerHTML = '<main class="main"><section></section></main>';
    const root = document.querySelector("main") as HTMLElement;
    const pane = root.querySelector("section") as HTMLElement;
    Object.defineProperty(pane, "scrollHeight", { get: () => 150 });
    Object.defineProperty(pane, "clientHeight", { get: () => 100 });
    let t = 0;
    restoreScroll(
      () => root,
      [{ path: [0], top: 300 }],
      () => t,
    );
    frames.shift()?.(0);
    t = 5000;
    frames.shift()?.(0);
    // jsdom doesn't clamp; a browser stops at the bottom, the nearest offset.
    expect(pane.scrollTop).toBe(300);
    expect(frames).toHaveLength(0);
  });
});
