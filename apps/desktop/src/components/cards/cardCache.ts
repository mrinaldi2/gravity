// The card lookups behind the links (UX-035 §3): every id asked for in one
// tick goes in one `item_cards_get`, an answer is kept for 60 s, and a board
// push for a card drops its copy so the next hover reads it again.

import type { DaemonApi } from "../../protocol/api";
import type { ItemCardEntry } from "../../protocol/itemCards";

/** How long an answer is shown without asking again. */
export const FRESH_MS = 60_000;

export interface Cached {
  readonly entry: ItemCardEntry;
  /** When it was answered (ms). */
  readonly at: number;
}

export class CardCache {
  private readonly entries = new Map<string, Cached>();
  /** Every answer seen, for "Last seen as" once a computer is offline. */
  private readonly titles = new Map<string, string>();
  private readonly pending = new Set<string>();
  private readonly inflight = new Set<string>();
  private readonly listeners = new Set<() => void>();
  private scheduled = false;
  private version = 0;

  constructor(
    private readonly api: DaemonApi,
    private readonly now: () => number = Date.now,
  ) {}

  /** Whether the service can look cards up at all. */
  get supported(): boolean {
    return this.api.capabilities.includes("item_cards");
  }

  get(id: string): Cached | undefined {
    return this.entries.get(id);
  }

  /** The last title seen for `id`, from any earlier answer. */
  lastTitle(id: string): string | undefined {
    return this.titles.get(id);
  }

  /** Asks for `id` unless a fresh answer or a request is already there. */
  want(id: string): void {
    const cached = this.entries.get(id);
    const fresh = cached !== undefined && this.now() - cached.at < FRESH_MS;
    if (fresh || this.inflight.has(id) || !this.supported) {
      return;
    }
    this.pending.add(id);
    if (!this.scheduled) {
      this.scheduled = true;
      queueMicrotask(() => void this.flush());
    }
  }

  /** Drops `id`'s copy: the card changed. */
  invalidate(id: string): void {
    if (this.entries.delete(id)) {
      this.emit();
    }
  }

  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };

  /** Changes on every answer, for `useSyncExternalStore`. */
  snapshot = (): number => this.version;

  private async flush(): Promise<void> {
    this.scheduled = false;
    const ids = [...this.pending];
    this.pending.clear();
    if (ids.length === 0) {
      return;
    }
    for (const id of ids) {
      this.inflight.add(id);
    }
    try {
      const reply = await this.api.request({ type: "item_cards_get", ids }, "item_cards");
      const at = this.now();
      for (const entry of reply.cards) {
        this.entries.set(entry.id, { entry, at });
        if (entry.card?.title) {
          this.titles.set(entry.id, entry.card.title);
        }
      }
    } catch {
      // Unanswered: the link says so after its wait, and the next hover asks again.
    } finally {
      for (const id of ids) {
        this.inflight.delete(id);
      }
      this.emit();
    }
  }

  private emit(): void {
    this.version += 1;
    for (const listener of this.listeners) {
      listener();
    }
  }
}
