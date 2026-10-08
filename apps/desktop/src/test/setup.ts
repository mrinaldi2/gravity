import "@testing-library/jest-dom/vitest";
import { cleanup } from "@testing-library/react";
import { afterEach, beforeEach, vi } from "vitest";

// jsdom implements neither of these; both are fire-and-forget in the UI.
Element.prototype.scrollIntoView = function scrollIntoView(): void {
  // no-op
};
globalThis.requestAnimationFrame = (callback: FrameRequestCallback): number => {
  callback(0);
  return 0;
};

/** A `Storage` held in memory, so a test never sees what another one saved. */
class MemoryStorage implements Storage {
  readonly #items = new Map<string, string>();

  get length(): number {
    return this.#items.size;
  }

  clear(): void {
    this.#items.clear();
  }

  getItem(key: string): string | null {
    return this.#items.get(key) ?? null;
  }

  key(index: number): string | null {
    return [...this.#items.keys()][index] ?? null;
  }

  removeItem(key: string): void {
    this.#items.delete(key);
  }

  setItem(key: string, value: string): void {
    this.#items.set(key, String(value));
  }
}

// Whether jsdom's localStorage is reachable depends on the Node version: from
// Node 25, Node's own `localStorage` getter shadows it and returns undefined
// without --localstorage-file. Saved view state (board filters, pins) then
// persists between tests on one Node and not on another (H-177). Every test
// starts on an empty store; a test that stubs its own still wins.
beforeEach(() => {
  vi.stubGlobal("localStorage", new MemoryStorage());
});

afterEach(() => {
  cleanup();
});
