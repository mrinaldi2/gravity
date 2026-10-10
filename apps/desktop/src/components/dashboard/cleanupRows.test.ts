import { describe, expect, it, vi } from "vitest";
import { sizeText } from "../../protocol/cleanup";
import type { AttentionRowJson } from "../../protocol/dashboard";
import { FAILED_ROW, HELD_ROW, LOW_ROW } from "../../test/cleanupFixtures";
import { attentionRow } from "./attentionRows";
import { cleanupWords } from "./cleanupText";

const opener = document.createElement("button");
const click = { currentTarget: opener } as never;

function actions(cleaningUp: string | null = null) {
  return {
    onItem: vi.fn<(itemId: string, opener: HTMLElement) => void>(),
    onDecideCleanup: vi.fn<(row: AttentionRowJson, opener: HTMLElement) => void>(),
    onCleanUp: vi.fn<(machine: string) => void>(),
    cleaningUp,
  };
}

describe("Needs you: cleanups and disk (H-275 AC3, AC4; UX-055)", () => {
  it("words a held cleanup from its fields, with no path, and opens the choice", () => {
    const a = actions();
    const view = attentionRow(HELD_ROW, a);
    expect(view?.title).toBe("Desktop Dev's worktree on imac is kept: 1 uncommitted file");
    expect(view?.meta).toBe("#42 · its changes are saved in the salvage folder · held 3 days");
    expect(view?.action).toBe("Decide…");
    expect(view?.label).toBe("Decide: Desktop Dev's worktree on imac");
    view?.onAction?.(click);
    expect(a.onDecideCleanup).toHaveBeenCalledWith(HELD_ROW, opener);
  });

  it("words a failed cleanup and one in no bot's folder", () => {
    const view = attentionRow(FAILED_ROW, actions());
    expect(view?.title).toBe("Couldn't remove iOS Dev's worktree on win-pc: the folder is in use");
    expect(view?.meta).toBe("It's tried again in the next daily cleanup. Keep it to stop asking.");
    const orphan = { ...HELD_ROW, cleanup: { ...HELD_ROW.cleanup, bot: undefined, unpushed: 2 } };
    expect(cleanupWords(orphan).title).toBe(
      "A worktree on imac is kept: 1 uncommitted file and 2 unpushed commits",
    );
  });

  it("keeps an older service's own title", () => {
    const old = { ...HELD_ROW, cleanup: undefined, title: "Cleanup held on imac: x" };
    expect(attentionRow(old, actions())?.title).toBe("Cleanup held on imac: x");
  });

  it("offers no choice without the job, or without a connection", () => {
    expect(
      attentionRow({ ...HELD_ROW, cleanup_job_id: undefined }, actions())?.action,
    ).toBeUndefined();
    const offline = { onItem: vi.fn<(itemId: string, opener: HTMLElement) => void>() };
    expect(attentionRow(HELD_ROW, offline)?.action).toBeUndefined();
  });

  it("offers Clean up on a computer low on disk, busy while it runs", () => {
    const a = actions();
    const view = attentionRow(LOW_ROW, a);
    expect(view?.action).toBe("Clean up");
    expect(view?.label).toBe("Clean up mac");
    view?.onAction?.(click);
    expect(a.onCleanUp).toHaveBeenCalledWith("mac");
    const busy = attentionRow(LOW_ROW, actions("mac"));
    expect(busy?.disabled).toBe(true);
    expect(busy?.meta).toBe("Cleaning up…");
  });

  it("says sizes the way the owner reads them", () => {
    expect(sizeText(14_000_000_000)).toBe("14.0 GB");
    expect(sizeText(300_000_000)).toBe("300 MB");
    expect(sizeText(120_000_000_000)).toBe("120 GB");
    expect(sizeText(4_000)).toBe("4 KB");
  });
});
