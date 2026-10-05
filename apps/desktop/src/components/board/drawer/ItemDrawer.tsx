// The item drawer (U4, H-018 §3.6): a 440px right drawer over the board or
// the dashboard. The state stepper, a meta line, the next step as the guard
// check turns it into guidance, then Overview, Links and Activity. Esc
// closes it; ↑/↓ move to the neighbouring card when the board offers them.

import { useEffect, useState } from "react";
import type { ReactElement } from "react";
import type { BoardApi } from "../../../protocol/board";
import type { Bot } from "../../../protocol/entities";
import type { BoardColumn } from "../../../protocol/gen/hermes/board/v1/board_pb";
import type { ItemDetail } from "../../../protocol/gen/hermes/board/v1/requests_pb";
import type { Anchor } from "../useBoardMoves";
import { DrawerHeader, MetaLine, NextStepView, Stepper } from "./DrawerParts";
import { Activity, Links, Overview } from "./DrawerTabs";
import { nextStep } from "./drawerText";
import { useItemDetail } from "./useItemDetail";

export interface ItemDrawerProps {
  readonly api: BoardApi;
  readonly itemId: string;
  /** The card's version on the board: a push that moves it reads again. */
  readonly version?: bigint;
  readonly columns: readonly BoardColumn[];
  readonly bots: readonly Bot[];
  /** Whether this connection may comment (the control grant). */
  readonly canComment: boolean;
  readonly onClose: () => void;
  /** Opens "Move to…" below the button; absent where moves aren't offered. */
  readonly onMove?: (anchor: Anchor) => void;
  /** Steps to the previous (-1) or next (1) card in the column. */
  readonly onStep?: (direction: -1 | 1) => void;
  /** The clock, for "Doing for 31h"; tests and stories pin it. */
  readonly now?: () => number;
  /** The tab it opens on; Overview unless a story shows another. */
  readonly initialTab?: Tab;
}

type Tab = "overview" | "links" | "activity";

/** "bot:<id>", "user", "device:…": who did something, in words. */
function whoNamer(bots: readonly Bot[]): (actor: string) => string {
  return (actor) => {
    if (actor === "user" || actor === "owner" || actor.startsWith("device:")) {
      return "You";
    }
    const id = actor.startsWith("bot:") ? actor.slice(4) : actor;
    return bots.find((b) => b.id === id)?.name ?? "a bot";
  };
}

function useDrawerKeys(onClose: () => void, onStep?: (direction: -1 | 1) => void): void {
  useEffect(() => {
    const onKey = (event: KeyboardEvent): void => {
      const typing =
        event.target instanceof HTMLElement &&
        (event.target.tagName === "TEXTAREA" || event.target.tagName === "INPUT");
      if (event.key === "Escape") {
        // A menu or dialog over the drawer takes its own Esc first.
        if (document.querySelector('[role="menu"], [role="dialog"]') === null) {
          onClose();
        }
      } else if (!typing && onStep && (event.key === "ArrowUp" || event.key === "ArrowDown")) {
        event.preventDefault();
        onStep(event.key === "ArrowUp" ? -1 : 1);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose, onStep]);
}

export default function ItemDrawer(props: ItemDrawerProps): ReactElement {
  const { api, itemId, columns, bots } = props;
  const { detail, check, error, comment } = useItemDetail(api, itemId, props.version);
  useDrawerKeys(props.onClose, props.onStep);
  const who = whoNamer(bots);
  const columnName = (key: string): string => columns.find((c) => c.key === key)?.name ?? key;
  const item = detail?.item;
  const column = columns.find((c) => c.key === item?.columnKey);
  const now = props.now ?? Date.now;

  return (
    <aside className="item-drawer" aria-label={`Item ${itemId}`}>
      <DrawerHeader itemId={itemId} item={item} onMove={props.onMove} onClose={props.onClose} />
      {item === undefined ? (
        <p className="drawer-empty" role="status">
          {error === null ? "Loading…" : `Couldn't load ${itemId}: ${error}`}
        </p>
      ) : (
        <div className="drawer-body">
          <h2 className="drawer-title">{item.title}</h2>
          <Stepper column={column} />
          <MetaLine item={item} column={column} who={who} now={now()} />
          <NextStepView
            step={nextStep(columns, column, check)}
            assignee={item.assignee ? who(`bot:${item.assignee}`) : null}
          />
          {detail ? (
            <DrawerTabs
              detail={detail}
              initialTab={props.initialTab}
              who={who}
              columnName={columnName}
              onComment={props.canComment ? comment : undefined}
            />
          ) : null}
        </div>
      )}
    </aside>
  );
}

function DrawerTabs(props: {
  readonly detail: ItemDetail;
  readonly initialTab: Tab | undefined;
  readonly who: (actor: string) => string;
  readonly columnName: (key: string) => string;
  readonly onComment?: (body: string) => Promise<void>;
}): ReactElement {
  const { detail, who } = props;
  const [tab, setTab] = useState<Tab>(props.initialTab ?? "overview");
  const tabs: readonly (readonly [Tab, string])[] = [
    ["overview", "Overview"],
    ["links", `Links ${detail.links.length}`],
    ["activity", `Activity ${detail.history.length + detail.comments.length}`],
  ];
  return (
    <>
      <div className="drawer-tabs" role="tablist" aria-label="Item">
        {tabs.map(([id, label]) => (
          <button
            key={id}
            type="button"
            role="tab"
            aria-selected={tab === id}
            className={tab === id ? "on" : ""}
            onClick={() => setTab(id)}
          >
            {label}
          </button>
        ))}
      </div>
      <div role="tabpanel">
        {tab === "overview" ? <Overview detail={detail} who={who} /> : null}
        {tab === "links" ? <Links links={detail.links} /> : null}
        {tab === "activity" ? (
          <Activity
            detail={detail}
            who={who}
            columnName={props.columnName}
            onComment={props.onComment}
          />
        ) : null}
      </div>
    </>
  );
}
