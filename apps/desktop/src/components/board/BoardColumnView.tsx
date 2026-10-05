import { ChevronLeft, ChevronRight, Filter } from "lucide-react";
import type { DragEvent, HTMLAttributes, ReactElement, ReactNode } from "react";
import type { Bot } from "../../protocol/entities";
import type { BoardColumn } from "../../protocol/gen/hermes/board/v1/board_pb";
import BotAvatar from "../BotAvatar";
import ColumnLimit from "./ColumnLimit";
import type { WipSummary } from "./wip";
import { countText, wipDescription } from "./wip";

/** How a column reads while a card is dragged over the board. */
export type DropState =
  | { readonly kind: "idle" }
  | { readonly kind: "allowed" }
  | { readonly kind: "refused"; readonly chip: string };

interface DropHandlers {
  readonly onDragEnter: () => void;
  readonly onDrop: (anchor: { readonly x: number; readonly y: number }) => void;
}

interface BoardColumnViewProps extends DropHandlers {
  readonly column: BoardColumn;
  readonly summary: WipSummary;
  readonly drop: DropState;
  readonly botsById: ReadonlyMap<string, Bot>;
  /** Folds the column back into a rail (Inbox only). */
  readonly onCollapse?: () => void;
  /** The owner's limit editor; absent for anyone else. */
  readonly onSetLimit?: (limit: number | undefined) => Promise<void>;
  readonly children: ReactNode;
}

type DropTargetProps = Pick<HTMLAttributes<HTMLElement>, "onDragEnter" | "onDragOver" | "onDrop">;

function dragProps(handlers: DropHandlers, active: boolean): DropTargetProps {
  if (!active) {
    return {};
  }
  return {
    onDragEnter: handlers.onDragEnter,
    onDragOver: (event: DragEvent<HTMLElement>) => {
      // Every column takes the drop: a refused one answers with the popover.
      event.preventDefault();
    },
    onDrop: (event: DragEvent<HTMLElement>) => {
      event.preventDefault();
      const rect = event.currentTarget.getBoundingClientRect();
      handlers.onDrop({ x: rect.left + 8, y: Math.min(event.clientY, rect.bottom - 8) });
    },
  };
}

function WipBadge({ summary }: { readonly summary: WipSummary }): ReactElement | null {
  if (summary.state === "full") {
    return (
      <span className="board-wip board-wip-full">
        <span aria-hidden="true">●</span> Full
      </span>
    );
  }
  if (summary.state === "over") {
    return (
      <span className="board-wip board-wip-over">
        <span aria-hidden="true">▲</span> Over
      </span>
    );
  }
  return null;
}

/** Filters hiding cards here: the count reads "1 of 17", marked (H-101). */
function FilterMark({ summary }: { readonly summary: WipSummary }): ReactElement | null {
  if (summary.shown === summary.total) {
    return null;
  }
  const hidden = summary.total - summary.shown;
  return (
    <span
      className="board-column-filtered"
      title={`Filters hide ${hidden} ${hidden === 1 ? "card" : "cards"} here`}
    >
      <Filter size={12} aria-hidden="true" />
      <span className="visually-hidden">Filtered:</span>
    </span>
  );
}

function HeaderCount({ summary }: { readonly summary: WipSummary }): ReactElement {
  if (summary.limit === undefined) {
    return <span className="board-column-count">{countText(summary)}</span>;
  }
  if (summary.perAssignee) {
    return (
      <span className="board-column-count">
        {countText(summary)} · {summary.limit} per bot
      </span>
    );
  }
  return (
    <span className="board-column-count">
      {summary.shown === summary.total
        ? `${summary.total}/${summary.limit}`
        : `${countText(summary)} · limit ${summary.limit}`}
    </span>
  );
}

/** One board column: a WIP header and its cards (H-018 §3.1, §3.4). */
export default function BoardColumnView(props: BoardColumnViewProps): ReactElement {
  const { column, summary, drop, botsById } = props;
  const description = wipDescription(column.name, summary);
  const className = [
    "board-column",
    `board-column-${summary.state}`,
    drop.kind === "allowed" ? "board-column-allowed" : "",
    drop.kind === "refused" ? "board-column-refused" : "",
  ]
    .filter(Boolean)
    .join(" ");

  return (
    <section
      className={className}
      aria-label={description}
      data-column-key={column.key}
      {...dragProps(props, drop.kind !== "idle")}
    >
      <header className="board-column-header" title={description}>
        <div className="board-column-heading">
          <h3 className="board-column-name">{column.name}</h3>
          <FilterMark summary={summary} />
          <HeaderCount summary={summary} />
          <WipBadge summary={summary} />
          {props.onSetLimit === undefined ? null : (
            <ColumnLimit
              columnName={column.name}
              limit={column.wipLimit}
              onSave={props.onSetLimit}
            />
          )}
          {props.onCollapse === undefined ? null : (
            <button
              type="button"
              className="board-column-collapse"
              aria-label={`Collapse ${column.name}`}
              title={`Collapse ${column.name}`}
              onClick={props.onCollapse}
            >
              <ChevronLeft size={14} aria-hidden="true" />
            </button>
          )}
        </div>
        {summary.loads.length === 0 ? null : (
          <ul className="board-column-loads" aria-label="Items per bot">
            {summary.loads.map((load) => {
              const bot = botsById.get(load.botId);
              const name = bot?.name ?? load.botId;
              return (
                <li key={load.botId} className="board-column-load" title={name}>
                  <BotAvatar avatar={bot?.avatar ?? ""} name={name} id={load.botId} size="sm" />
                  <span>
                    <span className="visually-hidden">{name} </span>
                    {load.count}/{summary.limit}
                  </span>
                </li>
              );
            })}
          </ul>
        )}
        {drop.kind === "refused" ? <p className="board-column-chip">{drop.chip}</p> : null}
      </header>
      <div className="board-column-cards">{props.children}</div>
    </section>
  );
}

interface InboxRailProps extends DropHandlers {
  readonly column: BoardColumn;
  readonly summary: WipSummary;
  readonly drop: DropState;
  readonly onExpand: () => void;
}

/** Inbox as a collapsed 48px rail: triage, not flow (H-018 §3.1). */
export function InboxRail(props: InboxRailProps): ReactElement {
  const { column, summary, drop } = props;
  return (
    <section
      className={`board-rail${drop.kind === "refused" ? " board-column-refused" : ""}${drop.kind === "allowed" ? " board-column-allowed" : ""}`}
      aria-label={wipDescription(column.name, summary)}
      data-column-key={column.key}
      {...dragProps(props, drop.kind !== "idle")}
    >
      <button
        type="button"
        className="board-rail-button"
        aria-expanded="false"
        title={`Show ${column.name}`}
        onClick={props.onExpand}
      >
        <span className="board-rail-count">{countText(summary)}</span>
        <ChevronRight size={14} aria-hidden="true" />
        <span className="board-rail-name">{column.name}</span>
      </button>
    </section>
  );
}
