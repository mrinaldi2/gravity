// The board's client state: a snapshot kept live by `board_event` pushes,
// following B4's sync rule — apply `seq == last + 1`, ignore `seq <= last`,
// and refetch on anything else (a lower number means the daemon restarted).

import type { BoardApi } from "../../protocol/board";
import { boardCall } from "../../protocol/board";
import { DaemonError } from "../../protocol/connection";
import type {
  BoardColumn,
  BoardSettings,
  ItemCard,
  ProjectRole,
} from "../../protocol/gen/hermes/board/v1/board_pb";
import { BoardEventKind } from "../../protocol/gen/hermes/board/v1/requests_pb";
import type { BoardEvent, BoardSnapshot } from "../../protocol/gen/hermes/board/v1/requests_pb";

export interface BoardModel {
  readonly settings: BoardSettings | undefined;
  /** Every column, hidden ones included, in board order. */
  readonly columns: readonly BoardColumn[];
  readonly cards: ReadonlyMap<string, ItemCard>;
  readonly roles: readonly ProjectRole[];
  /** The last change this model includes. */
  readonly seq: bigint;
}

function byOrd(columns: readonly BoardColumn[]): BoardColumn[] {
  // `sort` on a fresh copy, not `toSorted`: the build targets ES2021.
  const sorted = [...columns];
  // oxlint-disable-next-line unicorn/no-array-sort
  sorted.sort((a, b) => a.ord - b.ord);
  return sorted;
}

export function fromSnapshot(snapshot: BoardSnapshot): BoardModel {
  return {
    settings: snapshot.settings,
    columns: byOrd(snapshot.columns),
    cards: new Map(snapshot.cards.map((card) => [card.id, card])),
    roles: snapshot.roles,
    seq: snapshot.seq,
  };
}

export type EventOutcome =
  | { readonly kind: "applied"; readonly model: BoardModel }
  | { readonly kind: "ignored" }
  | { readonly kind: "resync" };

const IGNORED: EventOutcome = { kind: "ignored" };
const RESYNC: EventOutcome = { kind: "resync" };

/** Applies one push to `model`, or says why it can't. */
export function applyEvent(model: BoardModel, event: BoardEvent): EventOutcome {
  if (event.seq <= model.seq) {
    return IGNORED;
  }
  if (event.seq !== model.seq + 1n) {
    return RESYNC;
  }
  const cards = new Map(model.cards);
  switch (event.kind) {
    case BoardEventKind.ITEM_UPSERTED:
    case BoardEventKind.ITEM_MOVED:
      if (event.card === undefined) {
        return RESYNC;
      }
      cards.set(event.itemId, event.card);
      break;
    case BoardEventKind.ITEM_REMOVED:
      cards.delete(event.itemId);
      break;
    case BoardEventKind.COLUMNS_CHANGED:
    case BoardEventKind.SETTINGS_CHANGED:
    case BoardEventKind.RESYNC:
      return RESYNC;
    default:
      // An unknown kind from a newer daemon still takes its place in the
      // sequence, so the next push isn't read as a gap.
      break;
  }
  return { kind: "applied", model: { ...model, cards, seq: event.seq } };
}

export type BoardStatus =
  | { readonly kind: "loading" }
  | { readonly kind: "ready"; readonly model: BoardModel }
  | { readonly kind: "failed"; readonly code: string; readonly message: string };

/**
 * Watches one project's board. Pushes that arrive while a snapshot is in
 * flight are held and replayed on top of it, so a refetch never opens a gap
 * of its own.
 */
export class BoardSync {
  private readonly api: BoardApi;
  private readonly projectId: string;
  private readonly onChange: (status: BoardStatus) => void;
  private model: BoardModel | null = null;
  private loading = false;
  private held: BoardEvent[] = [];
  private stopped = false;
  private unsubscribe: (() => void) | null = null;

  constructor(api: BoardApi, projectId: string, onChange: (status: BoardStatus) => void) {
    this.api = api;
    this.projectId = projectId;
    this.onChange = onChange;
  }

  start(): void {
    this.unsubscribe = this.api.onBoardEvent((event) => {
      this.receive(event);
    });
    void this.load("boardWatch");
  }

  stop(): void {
    this.stopped = true;
    this.unsubscribe?.();
    this.unsubscribe = null;
    if (this.model !== null) {
      this.api.board({ case: "boardUnwatch", value: { projectId: this.projectId } }).catch(() => {
        // The watch ends with the connection anyway.
      });
    }
  }

  /**
   * Refetches the snapshot, e.g. after a move found the board stale. Before
   * the first snapshot there is no watch yet, so that is retried instead.
   */
  refresh(): void {
    if (!this.loading) {
      this.onChange(
        this.model === null ? { kind: "loading" } : { kind: "ready", model: this.model },
      );
      void this.load(this.model === null ? "boardWatch" : "boardGet");
    }
  }

  private async load(arm: "boardWatch" | "boardGet"): Promise<void> {
    this.loading = true;
    let snapshot: BoardSnapshot;
    try {
      const value = { projectId: this.projectId };
      snapshot = await boardCall(
        this.api,
        arm === "boardWatch" ? { case: arm, value } : { case: arm, value },
        "board",
      );
    } catch (error) {
      this.loading = false;
      this.held = [];
      if (!this.stopped) {
        this.onChange(
          error instanceof DaemonError
            ? { kind: "failed", code: error.code, message: error.message }
            : { kind: "failed", code: "internal", message: String(error) },
        );
      }
      return;
    }
    if (this.stopped) {
      return;
    }
    this.model = fromSnapshot(snapshot);
    this.loading = false;
    const held = [...this.held];
    // oxlint-disable-next-line unicorn/no-array-sort
    held.sort((a, b) => (a.seq < b.seq ? -1 : a.seq > b.seq ? 1 : 0));
    this.held = [];
    for (const event of held) {
      if (!this.apply(event)) {
        return;
      }
    }
    this.onChange({ kind: "ready", model: this.model });
  }

  private receive(event: BoardEvent): void {
    if (this.stopped || event.projectId !== this.projectId) {
      return;
    }
    if (this.loading) {
      this.held.push(event);
      return;
    }
    if (this.model !== null && this.apply(event)) {
      this.onChange({ kind: "ready", model: this.model });
    }
  }

  /** Applies `event`; false when it started a refetch instead. */
  private apply(event: BoardEvent): boolean {
    if (this.model === null) {
      return false;
    }
    const outcome = applyEvent(this.model, event);
    if (outcome.kind === "resync") {
      void this.load("boardGet");
      return false;
    }
    if (outcome.kind === "applied") {
      this.model = outcome.model;
    }
    return true;
  }
}
