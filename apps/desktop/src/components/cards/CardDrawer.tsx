// The card drawer over any view (UX-035 §4): the board's item drawer lifted
// to app level, so a card id clicked in chat, Activity, Needs you or a
// release opens over it and the view underneath keeps its scroll. A card
// opened from inside the drawer stacks on it; `‹ Back` or ⌘[ returns, and
// on the first card closes. Esc always closes, and focus goes back to the
// link that opened it. "Open on the board" is the one action that navigates.

import { useCallback, useEffect, useRef, useState } from "react";
import type { ReactElement } from "react";
import { useLatestRef } from "../../app/useLatestRef";
import type { DaemonApi } from "../../protocol/api";
import type { Bot } from "../../protocol/entities";
import { create } from "@bufbuild/protobuf";
import { BoardColumnSchema } from "../../protocol/gen/hermes/board/v1/board_pb";
import type { BoardColumn } from "../../protocol/gen/hermes/board/v1/board_pb";
import DashboardItem from "../dashboard/DashboardItem";
import type { CardLinksValue } from "./CardLinks";
import { useCardLinks } from "./CardLinks";

/** The card's column, named as its `item_cards` answer names it (UX-045). */
function knownColumn(links: CardLinksValue | null, id: string): BoardColumn | undefined {
  const entry = links?.cache.get(id)?.entry;
  const key = entry?.card?.columnKey;
  return key && entry.column_name
    ? create(BoardColumnSchema, { key, name: entry.column_name })
    : undefined;
}

/** A card in the drawer, with the project whose board keeps it. */
interface OpenCard {
  readonly id: string;
  readonly projectId: string;
}

export interface CardDrawerApi {
  readonly stack: readonly OpenCard[];
  readonly open: (id: string, projectId: string) => void;
  readonly back: () => void;
  /** Closes it; `refocus` returns focus to the link that opened it. */
  readonly close: (refocus?: boolean) => void;
}

/** How many cards the drawer's own history keeps. */
const STACK_LIMIT = 50;

export function useCardDrawer(): CardDrawerApi {
  const [stack, setStack] = useState<readonly OpenCard[]>([]);
  const opener = useRef<HTMLElement | null>(null);
  const depth = useLatestRef(stack.length);

  const open = useCallback(
    (id: string, projectId: string): void => {
      if (depth.current === 0) {
        opener.current =
          document.activeElement instanceof HTMLElement ? document.activeElement : null;
      }
      setStack((s) => (s.at(-1)?.id === id ? s : [...s, { id, projectId }].slice(-STACK_LIMIT)));
    },
    [depth],
  );

  const close = useCallback((refocus = true): void => {
    setStack([]);
    if (refocus) {
      opener.current?.focus({ preventScroll: true });
    }
    opener.current = null;
  }, []);

  const back = useCallback((): void => {
    if (depth.current <= 1) {
      close();
    } else {
      setStack((s) => s.slice(0, -1));
    }
  }, [close, depth]);

  return { stack, open, back, close };
}

/** ⌘[ and the mouse's back button step back in the drawer while it's open. */
function useDrawerBack(open: boolean, back: () => void): void {
  useEffect(() => {
    if (!open) {
      return undefined;
    }
    const onKey = (event: KeyboardEvent): void => {
      if (event.metaKey && event.key === "[") {
        event.preventDefault();
        event.stopImmediatePropagation();
        back();
      }
    };
    const onMouse = (event: MouseEvent): void => {
      if (event.button === 3) {
        event.preventDefault();
        event.stopImmediatePropagation();
        back();
      }
    };
    window.addEventListener("keydown", onKey, { capture: true });
    window.addEventListener("mouseup", onMouse, { capture: true });
    return () => {
      window.removeEventListener("keydown", onKey, { capture: true });
      window.removeEventListener("mouseup", onMouse, { capture: true });
    };
  }, [open, back]);
}

interface CardDrawerProps {
  readonly drawer: CardDrawerApi;
  readonly client: DaemonApi;
  readonly bots: readonly Bot[];
  readonly canComment: boolean;
  /** Opens the card's project on its Board with the card selected. */
  readonly onOpenBoard: (card: OpenCard) => void;
}

export default function CardDrawer(props: CardDrawerProps): ReactElement | null {
  const { drawer } = props;
  const top = drawer.stack.at(-1);
  const before = drawer.stack.at(-2);
  const links = useCardLinks();
  useDrawerBack(top !== undefined, drawer.back);
  if (top === undefined) {
    return null;
  }
  return (
    <div className="card-drawer">
      <DashboardItem
        key={`${top.projectId}:${top.id}:${drawer.stack.length}`}
        api={props.client}
        projectId={top.projectId}
        itemId={top.id}
        bots={props.bots}
        canComment={props.canComment}
        onClose={() => drawer.close()}
        back={before ? { id: before.id, onBack: drawer.back } : undefined}
        column={knownColumn(links, top.id)}
        footer={
          <footer className="drawer-foot">
            <button
              type="button"
              className="btn btn-small"
              onClick={() => {
                drawer.close(false);
                props.onOpenBoard(top);
              }}
            >
              Open on the board
            </button>
          </footer>
        }
      />
    </div>
  );
}
