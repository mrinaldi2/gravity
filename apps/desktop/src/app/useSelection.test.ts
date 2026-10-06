import { act, renderHook } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { stubLocalStorage } from "../test/spies";
import type { Selection } from "./selection";
import type { UnreadApi } from "./useUnread";
import { useSelection } from "./useSelection";

const unread: UnreadApi = {
  bots: {},
  syncBots: vi.fn<UnreadApi["syncBots"]>(),
  bumpBot: vi.fn<UnreadApi["bumpBot"]>(),
  markSeen: vi.fn<UnreadApi["markSeen"]>(),
  advanceSeen: vi.fn<UnreadApi["advanceSeen"]>(),
  clearFor: vi.fn<UnreadApi["clearFor"]>(),
};

function render(): { current: ReturnType<typeof useSelection> } {
  return renderHook(() => useSelection(unread)).result;
}

function openProject(result: { current: ReturnType<typeof useSelection> }, next: Selection): void {
  act(() => {
    result.current.select(next);
  });
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("useSelection project windows", () => {
  it("opens a project on its Dashboard the first time", () => {
    stubLocalStorage();
    const result = render();

    openProject(result, { kind: "project", projectId: "p1" });

    expect(result.current.selection).toEqual({
      kind: "project",
      projectId: "p1",
      tab: "overview",
    });
  });

  it("reopens a project on the tab it last showed", () => {
    const store = stubLocalStorage();
    const result = render();

    openProject(result, { kind: "project", projectId: "p1", tab: "meetings" });
    openProject(result, { kind: "bot", botId: "b1" });
    openProject(result, { kind: "project", projectId: "p1" });

    expect(result.current.selection).toEqual({ kind: "project", projectId: "p1", tab: "meetings" });
    expect(store.get("hermes.project-tab.p1")).toBe("meetings");
  });

  it("remembers the tab per project", () => {
    stubLocalStorage({ "hermes.project-tab.p1": "board" });
    const result = render();

    openProject(result, { kind: "project", projectId: "p2" });

    expect(result.current.selection).toEqual({
      kind: "project",
      projectId: "p2",
      tab: "overview",
    });
  });

  it("ignores a stored tab that no longer exists", () => {
    stubLocalStorage({ "hermes.project-tab.p1": "timeline" });
    const result = render();

    openProject(result, { kind: "project", projectId: "p1" });

    expect(result.current.selection).toEqual({
      kind: "project",
      projectId: "p1",
      tab: "overview",
    });
  });
});
