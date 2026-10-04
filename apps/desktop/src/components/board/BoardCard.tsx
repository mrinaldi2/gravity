import { MoreHorizontal } from "lucide-react";
import type { DragEvent, KeyboardEvent, ReactElement } from "react";
import type { Bot } from "../../protocol/entities";
import type { ItemCard } from "../../protocol/gen/hermes/board/v1/board_pb";
import BotAvatar from "../BotAvatar";
import { platformWord, typeLabel, urgentPriority } from "./labels";
import type { Anchor } from "./useBoardMoves";

/** The label the daemon adds when the lead overrode a WIP limit. */
const WIP_OVERRIDE_LABEL = "wip-override";

interface BoardCardProps {
  readonly card: ItemCard;
  readonly columnName: string;
  readonly assignee: Bot | undefined;
  /** Null when this connection can't move items. */
  readonly onOpenMoveMenu: ((card: ItemCard, anchor: Anchor) => void) | null;
  readonly onDragStart: (card: ItemCard) => void;
  readonly onDragEnd: () => void;
  readonly dragging: boolean;
}

function assigneeName(card: ItemCard, assignee: Bot | undefined): string | null {
  return assignee?.name ?? card.assignee ?? null;
}

/** "H-017, Feature, Board and dashboard, Doing, Desktop Dev, blocked, stale" (H-018 §3.2). */
function cardAccessibleName(card: ItemCard, columnName: string, assignee: Bot | undefined): string {
  const parts = [card.id, typeLabel(card.type).word, card.title, columnName];
  parts.push(assigneeName(card, assignee) ?? "unassigned");
  const priority = urgentPriority(card.priority);
  if (priority !== null) {
    parts.push(`priority ${priority}`);
  }
  if (card.blocked) {
    parts.push("blocked");
  }
  if (card.stale) {
    parts.push("stale");
  }
  return parts.join(", ");
}

function anchorBelow(element: Element): Anchor {
  const rect = element.getBoundingClientRect();
  return { x: rect.left, y: rect.bottom + 4 };
}

/** One item on the board: at most six lines, each omitted when it has nothing to say. */
export default function BoardCard(props: BoardCardProps): ReactElement {
  const { card, columnName, assignee, onOpenMoveMenu, dragging } = props;
  const type = typeLabel(card.type);
  const priority = urgentPriority(card.priority);
  const name = assigneeName(card, assignee);
  const overWip = card.labels.includes(WIP_OVERRIDE_LABEL);

  const onKeyDown = (event: KeyboardEvent<HTMLElement>): void => {
    if (
      onOpenMoveMenu !== null &&
      event.target === event.currentTarget &&
      (event.key === "m" || event.key === "M") &&
      !event.metaKey &&
      !event.ctrlKey &&
      !event.altKey
    ) {
      event.preventDefault();
      onOpenMoveMenu(card, anchorBelow(event.currentTarget));
    }
  };

  const onDragStart = (event: DragEvent<HTMLElement>): void => {
    event.dataTransfer.effectAllowed = "move";
    event.dataTransfer.setData("text/plain", card.id);
    props.onDragStart(card);
  };

  const className = [
    "board-card",
    card.blocked ? "board-card-blocked" : "",
    dragging ? "board-card-dragging" : "",
  ]
    .filter(Boolean)
    .join(" ");

  // H-018 §3.2: a card is a focusable <article>; M opens "Move to…" on it.
  return (
    // oxlint-disable-next-line jsx-a11y/no-noninteractive-element-interactions, jsx-a11y/no-noninteractive-tabindex
    <article
      className={className}
      // oxlint-disable-next-line jsx-a11y/no-noninteractive-tabindex
      tabIndex={0}
      aria-label={cardAccessibleName(card, columnName, assignee)}
      data-item-id={card.id}
      draggable={onOpenMoveMenu !== null}
      onDragStart={onDragStart}
      onDragEnd={props.onDragEnd}
      onKeyDown={onKeyDown}
    >
      <div className="board-card-top">
        <span className="board-card-type">
          <span aria-hidden="true">{type.glyph}</span> {type.word}
        </span>
        <span className="board-card-sep" aria-hidden="true">
          ·
        </span>
        <span className="board-card-id">{card.id}</span>
        {priority === null ? null : (
          <span className={`board-card-priority board-card-${priority.toLowerCase()}`}>
            <span aria-hidden="true">‼</span> {priority}
          </span>
        )}
        {onOpenMoveMenu === null ? null : (
          <button
            type="button"
            className="board-card-more"
            aria-label={`Move ${card.id} to…`}
            title="Move to… (M)"
            aria-haspopup="menu"
            tabIndex={-1}
            onClick={(event) => {
              const article = event.currentTarget.closest("article");
              onOpenMoveMenu(card, anchorBelow(article ?? event.currentTarget));
            }}
          >
            <MoreHorizontal size={14} aria-hidden="true" />
          </button>
        )}
      </div>
      <div className="board-card-title">{card.title}</div>
      {card.platforms.length === 0 ? null : (
        <div className="board-card-chips">
          {card.platforms.map((platform) => (
            <span key={platform} className="board-chip">
              {platformWord(platform)}
            </span>
          ))}
        </div>
      )}
      {card.blocked ? (
        <div className="board-card-line board-card-blocked-line">
          <span aria-hidden="true">⛔</span> Blocked
        </div>
      ) : null}
      {overWip ? (
        <div className="board-card-line board-card-override">
          <span aria-hidden="true">⚑</span> Over WIP
        </div>
      ) : null}
      {card.acTotal > 0 ? (
        <div className="board-card-line board-card-meta">
          <span aria-hidden="true">☑</span> {card.acChecked}/{card.acTotal} AC
        </div>
      ) : null}
      <div className="board-card-foot">
        {name === null ? (
          <span className="board-card-unassigned">Unassigned</span>
        ) : (
          <span className="board-card-assignee">
            <BotAvatar
              avatar={assignee?.avatar ?? ""}
              name={name}
              id={card.assignee ?? name}
              size="sm"
            />
            <span className="board-card-assignee-name">{name}</span>
          </span>
        )}
        {card.stale ? (
          <span className="board-card-stale">
            <span aria-hidden="true">⏱</span> Stale
          </span>
        ) : null}
      </div>
    </article>
  );
}
