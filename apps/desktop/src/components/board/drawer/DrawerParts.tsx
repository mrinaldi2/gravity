// The item drawer's fixed parts (H-018 §3.6): the header with Move to…, the
// state stepper, the meta line and the next step.

import { X } from "lucide-react";
import type { ReactElement } from "react";
import type { BoardColumn, Item } from "../../../protocol/gen/hermes/board/v1/board_pb";
import { Priority, Size } from "../../../protocol/gen/hermes/board/v1/board_pb";
import { platformWord, typeLabel } from "../labels";
import type { Anchor } from "../useBoardMoves";
import type { NextStep } from "./drawerText";
import { STEPS, age } from "./drawerText";

export function DrawerHeader(props: {
  readonly itemId: string;
  readonly item: Item | undefined;
  readonly onMove?: (anchor: Anchor) => void;
  readonly onClose: () => void;
}): ReactElement {
  const type = props.item ? typeLabel(props.item.type) : null;
  const move = props.onMove;
  return (
    <header className="drawer-head">
      <span className="drawer-type">
        {type ? `${type.glyph} ${type.word}` : ""} · <span className="mono">{props.itemId}</span>
      </span>
      {move ? (
        <button
          type="button"
          className="btn btn-small"
          onClick={(event) => {
            const rect = event.currentTarget.getBoundingClientRect();
            move({ x: rect.left, y: rect.bottom + 4 });
          }}
        >
          Move to…
        </button>
      ) : null}
      <button
        type="button"
        className="drawer-close"
        aria-label="Close the item"
        onClick={props.onClose}
      >
        <X size={16} aria-hidden="true" />
      </button>
    </header>
  );
}

function stepGlyph(index: number, reached: number): string {
  if (index === reached) {
    return "◉";
  }
  return index < reached ? "●" : "○";
}

/** The canonical steps, the current one filled; words, not colour. */
export function Stepper({ column }: { readonly column: BoardColumn | undefined }): ReactElement {
  const reached = STEPS.findIndex((s) => s.category === column?.category);
  return (
    <ol className="drawer-stepper" aria-label="Where it stands">
      {STEPS.map((s, index) => (
        <li
          key={s.word}
          className={index === reached ? "now" : index < reached ? "past" : ""}
          aria-current={index === reached ? "step" : undefined}
        >
          <span aria-hidden="true">{stepGlyph(index, reached)}</span> {s.word}
        </li>
      ))}
    </ol>
  );
}

/** "Doing for 31h · Desktop Dev · P1 · M · desktop daemon". */
export function MetaLine(props: {
  readonly item: Item;
  readonly column: BoardColumn | undefined;
  readonly who: (actor: string) => string;
  readonly now: number;
}): ReactElement {
  const { item, column } = props;
  const parts = [
    column ? `${column.name} for ${age(item.stateEnteredAt, props.now)}` : item.columnKey,
    item.assignee ? props.who(`bot:${item.assignee}`) : "Unassigned",
    Priority[item.priority],
    item.size === undefined ? "" : Size[item.size],
    item.platforms.map(platformWord).join(" "),
  ];
  return (
    <p className="drawer-meta">
      {parts.filter(Boolean).join(" · ")}
      {item.blocked ? (
        <span className="drawer-blocked"> · ⛔ Blocked: {item.blocked.reason}</span>
      ) : null}
    </p>
  );
}

/** The guard check for the next forward move, as guidance (§3.6). */
export function NextStepView(props: {
  readonly step: NextStep;
  readonly assignee: string | null;
}): ReactElement | null {
  const { to, unmet } = props.step;
  if (to === undefined) {
    return null;
  }
  return (
    <section className="drawer-next" aria-label="Next step">
      {unmet.length === 0 ? (
        <p>
          Ready to move to {to.name}
          {props.assignee ? ` — waiting on ${props.assignee}` : ""}
        </p>
      ) : (
        <>
          <p>To reach {to.name}:</p>
          <ul>
            {unmet.map((u) => (
              <li key={u.code}>
                <span aria-hidden="true">✗</span> {u.text}
                {u.fix ? <span className="drawer-dim"> {u.fix}</span> : null}
              </li>
            ))}
          </ul>
        </>
      )}
    </section>
  );
}
