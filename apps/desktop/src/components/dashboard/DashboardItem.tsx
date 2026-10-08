// An item's drawer over the dashboard (U4): the same drawer the board opens,
// with the board's columns read once for its stepper and next step. Moves
// stay on the board, which "Open board" reaches.

import { useEffect, useState } from "react";
import type { ReactElement, ReactNode } from "react";
import type { BoardApi } from "../../protocol/board";
import { boardCall } from "../../protocol/board";
import type { Bot } from "../../protocol/entities";
import type { BoardColumn } from "../../protocol/gen/hermes/board/v1/board_pb";
import ItemDrawer from "../board/drawer/ItemDrawer";
import type { ItemDrawerProps } from "../board/drawer/ItemDrawer";

export default function DashboardItem(props: {
  readonly api: BoardApi;
  readonly projectId: string;
  readonly itemId: string;
  readonly bots: readonly Bot[];
  readonly canComment: boolean;
  readonly onClose: () => void;
  /** The item's commands for the owner (H-117), above its tabs. */
  readonly children?: ReactNode;
  /** Where the app-level card drawer (UX-035 §4) adds `‹ Back` and its footer. */
  readonly back?: ItemDrawerProps["back"];
  readonly footer?: ReactNode;
  /** The item's column as known elsewhere, named while the board's can't be read (UX-045). */
  readonly column?: BoardColumn;
}): ReactElement {
  const { api, projectId } = props;
  const [columns, setColumns] = useState<readonly BoardColumn[]>([]);
  useEffect(() => {
    let live = true;
    const load = async (): Promise<void> => {
      try {
        const board = await boardCall(api, { case: "boardGet", value: { projectId } }, "board");
        if (live) {
          setColumns(board.columns);
        }
      } catch {
        // Without the columns the drawer still shows the item, unstepped.
      }
    };
    void load();
    return () => {
      live = false;
    };
  }, [api, projectId]);
  return (
    <ItemDrawer
      api={api}
      itemId={props.itemId}
      columns={columns.length === 0 && props.column ? [props.column] : columns}
      bots={props.bots}
      canComment={props.canComment}
      onClose={props.onClose}
      back={props.back}
      footer={props.footer}
    >
      {props.children}
    </ItemDrawer>
  );
}
