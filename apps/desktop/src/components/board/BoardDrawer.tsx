// The board's item drawer (U4): the open card's drawer, with ↑/↓ stepping
// through the cards its column shows and "Move to…" opening the board's own
// move menu, so a move from the drawer runs the same guards as a drag.

import type { ReactElement } from "react";
import type { BoardApi } from "../../protocol/board";
import type { Bot } from "../../protocol/entities";
import type { BoardColumn, ItemCard } from "../../protocol/gen/hermes/board/v1/board_pb";
import ItemDrawer from "./drawer/ItemDrawer";
import type { Anchor } from "./useBoardMoves";

interface BoardDrawerProps {
  readonly api: BoardApi;
  readonly openId: string | null;
  /** The cards the board shows, in board order. */
  readonly shown: readonly ItemCard[];
  readonly columns: readonly BoardColumn[];
  readonly bots: readonly Bot[];
  readonly canControl: boolean;
  readonly onOpen: (id: string | null) => void;
  readonly onMove: ((card: ItemCard, anchor: Anchor) => void) | null;
}

export default function BoardDrawer(props: BoardDrawerProps): ReactElement | null {
  const { openId, shown } = props;
  if (openId === null) {
    return null;
  }
  const card = shown.find((c) => c.id === openId);
  const column = shown.filter((c) => c.columnKey === card?.columnKey);
  const step = (direction: -1 | 1): void => {
    const next = column[column.findIndex((c) => c.id === openId) + direction];
    if (next !== undefined) {
      props.onOpen(next.id);
    }
  };
  const move = props.onMove;
  return (
    <ItemDrawer
      key={openId}
      api={props.api}
      itemId={openId}
      version={card?.version}
      columns={props.columns}
      bots={props.bots}
      canComment={props.canControl}
      onClose={() => props.onOpen(null)}
      onMove={move && card ? (anchor) => move(card, anchor) : undefined}
      onStep={step}
    />
  );
}
