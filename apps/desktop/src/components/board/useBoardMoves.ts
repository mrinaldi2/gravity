import { useCallback, useRef, useState } from "react";
import type { AddToast } from "../../app/useToasts";
import type { BoardApi } from "../../protocol/board";
import { boardCall } from "../../protocol/board";
import type { BoardColumn, ItemCard, Unmet } from "../../protocol/gen/hermes/board/v1/board_pb";
import type { ColumnChecks } from "./moves";
import { columnChecks, MoveCheckCache, planMove } from "./moves";

/** Where a popover hangs: the bottom-left of the card or column it is about. */
export interface Anchor {
  readonly x: number;
  readonly y: number;
}

type MoveUi =
  | { readonly kind: "none" }
  | {
      readonly kind: "menu";
      readonly card: ItemCard;
      readonly anchor: Anchor;
      /** Null while the check is in flight. */
      readonly checks: ColumnChecks | null;
    }
  | {
      readonly kind: "input";
      readonly card: ItemCard;
      readonly to: BoardColumn;
      readonly anchor: Anchor;
      readonly needsReason: boolean;
      readonly needsOverride: boolean;
      readonly unmet: readonly Unmet[];
    }
  | {
      readonly kind: "refused";
      readonly card: ItemCard;
      readonly to: BoardColumn;
      readonly anchor: Anchor;
      readonly unmet: readonly Unmet[];
    };

const NONE: MoveUi = { kind: "none" };

interface Options {
  readonly api: BoardApi;
  readonly columns: readonly BoardColumn[];
  readonly refresh: () => void;
  readonly addToast: AddToast;
}

interface BoardMoves {
  readonly ui: MoveUi;
  /** The guards for every column `card` could move to (cached for drags). */
  readonly checksFor: (card: ItemCard) => Promise<ColumnChecks>;
  readonly openMenu: (card: ItemCard, anchor: Anchor) => void;
  /** Runs a move the way H-018 §3.3 says: go, ask for input, or show the refusal. */
  readonly requestMove: (card: ItemCard, toKey: string, anchor: Anchor) => void;
  readonly confirm: (reason: string, overrideReason: string) => void;
  readonly close: () => void;
}

function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

/** Returns focus to a card once the popover about it closes. */
function focusCard(id: string): void {
  const cards = document.querySelectorAll<HTMLElement>("[data-item-id]");
  [...cards].find((element) => element.dataset["itemId"] === id)?.focus();
}

export function useBoardMoves({ api, columns, refresh, addToast }: Options): BoardMoves {
  const [ui, setUi] = useState<MoveUi>(NONE);
  const cache = useRef(new MoveCheckCache());

  const checksFor = useCallback(
    async (card: ItemCard): Promise<ColumnChecks> => {
      const cached = cache.current.get(card.id, card.version, Date.now());
      if (cached !== undefined) {
        return cached;
      }
      const check = await boardCall(
        api,
        { case: "itemMoveCheck", value: { id: card.id } },
        "moveCheck",
      );
      const checks = columnChecks(check);
      cache.current.set(card.id, card.version, Date.now(), checks);
      return checks;
    },
    [api],
  );

  const close = useCallback((): void => {
    setUi((current) => {
      if (current.kind !== "none") {
        const { id } = current.card;
        queueMicrotask(() => {
          focusCard(id);
        });
      }
      return NONE;
    });
  }, []);

  const commit = useCallback(
    async (card: ItemCard, to: BoardColumn, anchor: Anchor, reason?: string, override?: string) => {
      try {
        const moved = await boardCall(
          api,
          {
            case: "itemMove",
            value: {
              id: card.id,
              to: to.key,
              expectedVersion: card.version,
              reason,
              overrideReason: override,
            },
          },
          "moved",
        );
        cache.current.clear();
        switch (moved.outcome.case) {
          case "refused":
            setUi({ kind: "refused", card, to, anchor, unmet: moved.outcome.value.unmet });
            return;
          case "conflict":
            addToast(
              "warn",
              `${card.id} changed meanwhile`,
              "The board is up to date again. Try the move once more.",
            );
            refresh();
            break;
          default:
            addToast("info", `${card.id} → ${to.name}`, "");
        }
      } catch (error) {
        addToast("error", `Couldn't move ${card.id}`, errorText(error));
      }
      close();
    },
    [api, addToast, refresh, close],
  );

  const requestMove = useCallback(
    (card: ItemCard, toKey: string, anchor: Anchor): void => {
      const to = columns.find((column) => column.key === toKey);
      if (to === undefined || toKey === card.columnKey) {
        return;
      }
      void (async () => {
        let checks: ColumnChecks;
        try {
          checks = await checksFor(card);
        } catch (error) {
          addToast("error", `Couldn't move ${card.id}`, errorText(error));
          return;
        }
        const plan = planMove(checks.get(toKey) ?? []);
        if (plan.kind === "go") {
          await commit(card, to, anchor);
        } else if (plan.kind === "input") {
          setUi({ ...plan, card, to, anchor });
        } else {
          setUi({ kind: "refused", card, to, anchor, unmet: plan.unmet });
        }
      })();
    },
    [columns, checksFor, commit, addToast],
  );

  const openMenu = useCallback(
    (card: ItemCard, anchor: Anchor): void => {
      setUi({ kind: "menu", card, anchor, checks: null });
      void (async () => {
        try {
          const checks = await checksFor(card);
          setUi((current) =>
            current.kind === "menu" && current.card.id === card.id
              ? { ...current, checks }
              : current,
          );
        } catch (error) {
          setUi(NONE);
          addToast("error", `Couldn't check moves for ${card.id}`, errorText(error));
        }
      })();
    },
    [checksFor, addToast],
  );

  const confirm = useCallback(
    (reason: string, overrideReason: string): void => {
      if (ui.kind !== "input") {
        return;
      }
      void commit(
        ui.card,
        ui.to,
        ui.anchor,
        ui.needsReason ? reason : undefined,
        ui.needsOverride ? overrideReason : undefined,
      );
    },
    [ui, commit],
  );

  return { ui, checksFor, openMenu, requestMove, confirm, close };
}
