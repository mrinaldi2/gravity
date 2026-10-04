import { useEffect, useLayoutEffect, useRef, useState } from "react";
import type { FormEvent, KeyboardEvent, ReactElement, ReactNode } from "react";
import type { BoardColumn, ItemCard, Unmet } from "../../protocol/gen/hermes/board/v1/board_pb";
import OverlayShell from "../overlay/OverlayShell";
import type { ColumnChecks } from "./moves";
import { planMove, reasonChip } from "./moves";
import type { Anchor } from "./useBoardMoves";

const MARGIN = 8;

interface PopoverProps {
  readonly anchor: Anchor;
  readonly role: "menu" | "dialog";
  readonly label: string;
  readonly onClose: () => void;
  readonly className: string;
  readonly children: ReactNode;
}

/** A panel hung below a card or column, kept on screen; Escape or a press outside closes it. */
function Popover({
  anchor,
  role,
  label,
  onClose,
  className,
  children,
}: PopoverProps): ReactElement {
  const ref = useRef<HTMLDivElement | null>(null);
  const [place, setPlace] = useState<Anchor | null>(null);

  useEffect(() => {
    const onKeyDown = (event: globalThis.KeyboardEvent): void => {
      if (event.key === "Escape") {
        event.preventDefault();
        onClose();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => {
      window.removeEventListener("keydown", onKeyDown);
    };
  }, [onClose]);

  useLayoutEffect(() => {
    const element = ref.current;
    if (element === null) {
      return;
    }
    const { width, height } = element.getBoundingClientRect();
    setPlace({
      x: Math.max(MARGIN, Math.min(anchor.x, window.innerWidth - width - MARGIN)),
      y: Math.max(MARGIN, Math.min(anchor.y, window.innerHeight - height - MARGIN)),
    });
  }, [anchor]);

  return (
    <div
      className="menu-backdrop"
      role="presentation"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) {
          onClose();
        }
      }}
    >
      <div
        ref={ref}
        className={`board-popover ${className}`}
        role={role}
        aria-label={label}
        style={{
          left: place?.x ?? anchor.x,
          top: place?.y ?? anchor.y,
          visibility: place === null ? "hidden" : "visible",
        }}
      >
        {children}
      </div>
    </div>
  );
}

interface MoveMenuProps {
  readonly card: ItemCard;
  readonly anchor: Anchor;
  readonly columns: readonly BoardColumn[];
  readonly checks: ColumnChecks | null;
  readonly onChoose: (toKey: string) => void;
  readonly onClose: () => void;
}

/** "Move to…": every other column with ✓ or ✗, and the reason under each ✗ (H-018 §3.3). */
export function MoveMenu(props: MoveMenuProps): ReactElement {
  const { card, anchor, checks, onChoose, onClose } = props;
  const listRef = useRef<HTMLDivElement | null>(null);
  const targets = props.columns.filter((column) => column.key !== card.columnKey);

  useEffect(() => {
    if (checks !== null) {
      listRef.current?.querySelector<HTMLElement>("[role=menuitem]")?.focus();
    }
  }, [checks]);

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>): void => {
    const items = [...(listRef.current?.querySelectorAll<HTMLElement>("[role=menuitem]") ?? [])];
    const at = items.findIndex((item) => item === document.activeElement);
    let next: number | null = null;
    if (event.key === "ArrowDown") {
      next = (at + 1) % items.length;
    } else if (event.key === "ArrowUp") {
      next = (at - 1 + items.length) % items.length;
    } else if (event.key === "Home") {
      next = 0;
    } else if (event.key === "End") {
      next = items.length - 1;
    } else if (event.key === "Tab") {
      event.preventDefault();
      onClose();
      return;
    }
    if (next !== null) {
      event.preventDefault();
      items[next]?.focus();
    }
  };

  return (
    <Popover
      anchor={anchor}
      role="dialog"
      label={`Move ${card.id} to…`}
      onClose={onClose}
      className="board-move-menu"
    >
      <h2 className="board-popover-title">{`Move ${card.id} to…`}</h2>
      {checks === null ? (
        <p className="board-popover-note" role="status">
          Checking which moves are allowed…
        </p>
      ) : (
        <div
          ref={listRef}
          role="menu"
          tabIndex={-1}
          aria-label={`Move ${card.id} to`}
          onKeyDown={onKeyDown}
        >
          {targets.map((column) => {
            const unmet = checks.get(column.key) ?? [];
            const plan = planMove(unmet);
            const refused = plan.kind === "refused";
            const reason = refused ? reasonChip(unmet) : "";
            return (
              <button
                key={column.key}
                type="button"
                role="menuitem"
                tabIndex={-1}
                className={`board-move-item${refused ? " board-move-refused" : ""}`}
                aria-label={refused ? `${column.name}, can't move: ${reason}` : column.name}
                onClick={() => {
                  onChoose(column.key);
                }}
              >
                <span className="board-move-mark" aria-hidden="true">
                  {refused ? "✗" : "✓"}
                </span>
                <span className="board-move-name">{column.name}</span>
                {refused ? <span className="board-move-reason">{reason}</span> : null}
              </button>
            );
          })}
        </div>
      )}
    </Popover>
  );
}

interface RefusalPopoverProps {
  readonly card: ItemCard;
  readonly to: BoardColumn;
  readonly anchor: Anchor;
  readonly unmet: readonly Unmet[];
  readonly onClose: () => void;
}

/** "Can't move H-024 to Ready": every unmet guard, as the Hermes service words it (H-018 O2). */
export function RefusalPopover({
  card,
  to,
  anchor,
  unmet,
  onClose,
}: RefusalPopoverProps): ReactElement {
  const title = `Can't move ${card.id} to ${to.name}`;
  return (
    <Popover
      anchor={anchor}
      role="dialog"
      label={title}
      onClose={onClose}
      className="board-refusal"
    >
      <h2 className="board-popover-title">{title}</h2>
      <ul className="board-refusal-list">
        {unmet.map((u) => (
          <li key={`${u.code}:${u.text}`}>
            <span className="board-refusal-mark" aria-hidden="true">
              ✗
            </span>
            <span>
              <span className="board-refusal-text">{u.text}</span>
              {u.fix === undefined ? null : <span className="board-refusal-fix">{u.fix}</span>}
            </span>
          </li>
        ))}
      </ul>
      <div className="board-popover-actions">
        <button type="button" className="btn btn-small" autoFocus onClick={onClose}>
          OK
        </button>
      </div>
    </Popover>
  );
}

interface MoveDialogProps {
  readonly card: ItemCard;
  readonly to: BoardColumn;
  readonly needsReason: boolean;
  readonly needsOverride: boolean;
  readonly unmet: readonly Unmet[];
  readonly onConfirm: (reason: string, overrideReason: string) => void;
  readonly onCancel: () => void;
}

function textOf(unmet: readonly Unmet[], code: string): string {
  return unmet.find((u) => u.code === code)?.text ?? "";
}

/** A move that is allowed once the owner says why: a reason, a WIP override, or both. */
export function MoveDialog(props: MoveDialogProps): ReactElement {
  const { card, to, needsReason, needsOverride, unmet, onCancel } = props;
  const [reason, setReason] = useState("");
  const [override, setOverride] = useState("");
  const [sent, setSent] = useState(false);

  useEffect(() => {
    const onKeyDown = (event: globalThis.KeyboardEvent): void => {
      if (event.key === "Escape") {
        onCancel();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => {
      window.removeEventListener("keydown", onKeyDown);
    };
  }, [onCancel]);

  const ready =
    (!needsReason || reason.trim().length > 0) && (!needsOverride || override.trim().length > 0);
  const title = `Move ${card.id} to ${to.name}`;
  const onSubmit = (event: FormEvent): void => {
    event.preventDefault();
    if (ready && !sent) {
      setSent(true);
      props.onConfirm(reason.trim(), override.trim());
    }
  };

  return (
    <OverlayShell label={title} onClose={onCancel}>
      <form className="confirm-dialog board-move-dialog" onSubmit={onSubmit}>
        <h2 className="confirm-title">{title}</h2>
        {needsOverride ? (
          <label className="field">
            <span className="field-hint">{textOf(unmet, "wip.full")}</span>
            <span className="field-label">Override reason</span>
            <input
              type="text"
              value={override}
              autoFocus
              onChange={(event) => {
                setOverride(event.target.value);
              }}
            />
          </label>
        ) : null}
        {needsReason ? (
          <label className="field">
            <span className="field-hint">{textOf(unmet, "reason.required")}</span>
            <span className="field-label">Reason</span>
            <input
              type="text"
              value={reason}
              autoFocus={!needsOverride}
              onChange={(event) => {
                setReason(event.target.value);
              }}
            />
          </label>
        ) : null}
        <div className="confirm-actions">
          <button type="button" className="btn btn-small" onClick={onCancel}>
            Cancel
          </button>
          <button type="submit" className="btn btn-small btn-primary" disabled={!ready || sent}>
            {needsOverride ? "Move anyway" : "Move"}
          </button>
        </div>
      </form>
    </OverlayShell>
  );
}
