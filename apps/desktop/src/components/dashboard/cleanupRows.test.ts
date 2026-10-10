import { describe, expect, it, vi } from "vitest";
import { sizeText } from "../../protocol/cleanup";
import type { AttentionRowJson } from "../../protocol/dashboard";
import { attentionRow } from "./attentionRows";

const click = { currentTarget: document.createElement("button") } as never;

function actions(cleaningUp: string | null = null) {
  return {
    onItem: vi.fn<(itemId: string, opener: HTMLElement) => void>(),
    onDecideCleanup: vi.fn<(row: AttentionRowJson) => void>(),
    onCleanUp: vi.fn<(machine: string) => void>(),
    cleaningUp,
  };
}

const HELD: AttentionRowJson = {
  kind: "cleanup_held",
  id: "cleanup_held:mac:job-1",
  title: "Cleanup held on imac: gravity-wt-desktopdev-a: 1 uncommitted path(s) (salvaged to …)",
  cleanup_job_id: "job-1",
};

const LOW: AttentionRowJson = {
  kind: "disk_low",
  id: "disk_low:mac:mac",
  title: "mac is low on disk: 14.0 GB free; 9.1 GB is old build output",
  machine: "mac",
};

describe("Needs you: cleanups and disk (H-275 AC3, AC4)", () => {
  it("opens the choice for a held cleanup", () => {
    const a = actions();
    const view = attentionRow(HELD, a);
    expect(view?.title).toBe(HELD.title);
    expect(view?.action).toBe("Decide…");
    view?.onAction?.(click);
    expect(a.onDecideCleanup).toHaveBeenCalledWith(HELD);
  });

  it("offers no choice without the job, or without a connection", () => {
    expect(attentionRow({ ...HELD, cleanup_job_id: undefined }, actions())?.action).toBeUndefined();
    const offline = { onItem: vi.fn<(itemId: string, opener: HTMLElement) => void>() };
    expect(attentionRow(HELD, offline)?.action).toBeUndefined();
  });

  it("offers Clean up on a computer low on disk, busy while it runs", () => {
    const a = actions();
    const view = attentionRow(LOW, a);
    expect(view?.action).toBe("Clean up");
    expect(view?.label).toBe("Clean up mac");
    view?.onAction?.(click);
    expect(a.onCleanUp).toHaveBeenCalledWith("mac");
    const busy = attentionRow(LOW, actions("mac"));
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
