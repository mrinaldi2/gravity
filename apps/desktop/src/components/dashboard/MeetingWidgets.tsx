// Widgets 5 and 6 of the dashboard (H-018 §2.1, H-102): each meeting
// series with its next time, the meeting collecting now and the last one
// held; and the open action items, which the owner ticks, drops or
// promotes to the board. Every state carries a glyph and a word. Off the
// board's home both lists are empty, so they say where they are kept
// instead of "nothing" (UX-021, the H-112 pattern).

import { MoreHorizontal } from "lucide-react";
import { useState } from "react";
import type { ReactElement } from "react";
import type { Dashboard } from "../../protocol/dashboard";
import type { DashboardAction, MeetingRow, MeetingSummary } from "../../protocol/meetings";
import { OWNER } from "../../protocol/meetings";
import RowContextMenu from "../sidebar/RowContextMenu";
import { when } from "./needsYouText";
import { Widget } from "./Widgets";

function day(at: string): string {
  return new Date(at).toLocaleDateString([], {
    weekday: "short",
    month: "short",
    day: "numeric",
  });
}

/** The first line of the summary, or a skipped meeting's reason. */
function keyOutput(m: MeetingSummary): string {
  if (m.status === "skipped") {
    return m.skip_reason === null ? "skipped" : `skipped: ${m.skip_reason}`;
  }
  return m.summary.split("\n").find((line) => line.trim() !== "") ?? "no summary";
}

function Line(props: { readonly glyph: string; readonly children: string }): ReactElement {
  return (
    <span className="dash-row-meta">
      <span aria-hidden="true">{props.glyph} </span>
      {props.children}
    </span>
  );
}

function NextLine(props: { readonly row: MeetingRow }): ReactElement | null {
  const { row } = props;
  if (row.next_at !== null) {
    return <Line glyph="○">{`Next: ${when(row.next_at)}`}</Line>;
  }
  return row.series === null ? null : <Line glyph="⊘">Paused</Line>;
}

function MeetingLines(props: { readonly row: MeetingRow }): ReactElement {
  const { row } = props;
  const name = row.series?.name ?? row.collecting?.name ?? "Meeting";
  const held = row.last_held;
  return (
    <li className="dash-row">
      <span className="dash-glyph" aria-hidden="true">
        ◷
      </span>
      <span className="dash-row-text">
        <span className="dash-row-title">{name}</span>
        {row.collecting === null ? null : (
          <Line glyph="◐">
            {`Collecting · ${row.collecting.contributed} of ${row.collecting.attendee_count} contributed`}
          </Line>
        )}
        <NextLine row={row} />
        {held === null ? null : (
          <Line glyph={held.status === "skipped" ? "⊘" : "●"}>
            {`Last: ${day(held.closed_at ?? held.started_at ?? "")} · ${keyOutput(held)}`}
          </Line>
        )}
      </span>
    </li>
  );
}

/** Off the board's home: the computer holding it, and whether it answered. */
export interface OffHome {
  readonly home: string;
  readonly away: boolean;
}

/** Off-home the lists are empty; Needs you's note says the home can't be reached. */
export function offHomeOf(d: Pick<Dashboard, "home" | "needs_you_note">): OffHome | null {
  return d.home === null ? null : { home: d.home, away: Boolean(d.needs_you_note) };
}

function offHomeText(off: OffHome, what: string, reachable: string): string {
  return off.away ? `Can't reach ${off.home} right now, so ${what} can't be shown.` : reachable;
}

/** Widget 5. */
export function MeetingsWidget(props: {
  readonly rows: readonly MeetingRow[];
  /** The project's lead bot by name; null without one. */
  readonly leadName: string | null;
  readonly offHome: OffHome | null;
}): ReactElement {
  const off = props.offHome;
  const empty =
    off === null
      ? `No meetings set up yet. ${props.leadName ?? "Your lead bot"} schedules the regular ones, such as the stand-up and the retro.`
      : offHomeText(
          off,
          "meetings",
          `Meetings are kept on ${off.home}. Open The Hermes there to see them.`,
        );
  return (
    <Widget id="dash-meetings" title="Meetings">
      {props.rows.length === 0 ? (
        <p className="dash-empty">{empty}</p>
      ) : (
        <ul className="dash-rows">
          {props.rows.map((row) => (
            <MeetingLines key={row.series?.id ?? row.collecting?.id} row={row} />
          ))}
        </ul>
      )}
    </Widget>
  );
}

export interface ActionItemsProps {
  readonly actions: readonly DashboardAction[];
  readonly botName: (id: string) => string;
  /** The owner may change them (the control grant). */
  readonly canControl: boolean;
  readonly offHome: OffHome | null;
  readonly onDone: (actionId: string) => void;
  readonly onDrop: (actionId: string) => void;
  readonly onPromote: (actionId: string) => void;
  readonly onItem: (itemId: string, opener: HTMLElement) => void;
}

interface Menu {
  readonly action: DashboardAction;
  readonly x: number;
  readonly y: number;
}

function ActionMeta(props: {
  readonly action: DashboardAction;
  readonly owner: string;
  readonly onItem: (itemId: string, opener: HTMLElement) => void;
}): ReactElement {
  const a = props.action;
  return (
    <span className="dash-row-meta">
      {props.owner}
      {a.due_at === null ? "" : ` · due ${day(a.due_at)}`}
      {a.overdue ? (
        <strong className="dash-overdue">
          {" "}
          <span aria-hidden="true">‼</span> overdue
        </strong>
      ) : null}
      {a.meeting_name === null ? "" : ` · ${a.meeting_name}`}
      {a.item_id === null ? null : (
        <>
          {" · "}
          <button
            type="button"
            className="dash-inline-link"
            onClick={(event) => props.onItem(a.item_id ?? "", event.currentTarget)}
          >
            {a.item_id}
          </button>
        </>
      )}
    </span>
  );
}

function ActionRow(
  props: ActionItemsProps & {
    readonly action: DashboardAction;
    readonly onMenu: (menu: Menu) => void;
  },
): ReactElement {
  const a = props.action;
  return (
    <li className="dash-row">
      <input
        type="checkbox"
        className="dash-check"
        checked={false}
        disabled={!props.canControl}
        aria-label={`Mark done: ${a.text}`}
        onChange={() => props.onDone(a.id)}
      />
      <span className="dash-row-text">
        <span className="dash-row-title">{a.text}</span>
        <ActionMeta
          action={a}
          owner={a.owner === OWNER ? "You" : props.botName(a.owner)}
          onItem={props.onItem}
        />
      </span>
      {props.canControl ? (
        <button
          type="button"
          className="dash-row-menu"
          aria-label={`More actions for ${a.text}`}
          onClick={(event) => {
            const r = event.currentTarget.getBoundingClientRect();
            props.onMenu({ action: a, x: r.right, y: r.bottom });
          }}
        >
          <MoreHorizontal size={14} aria-hidden="true" />
        </button>
      ) : null}
    </li>
  );
}

function title(actions: readonly DashboardAction[]): string {
  if (actions.length === 0) {
    return "Action items";
  }
  const overdue = actions.filter((a) => a.overdue).length;
  return `Action items (${actions.length} open${overdue === 0 ? "" : `, ${overdue} overdue`})`;
}

/** Widget 6. */
export function ActionItemsWidget(props: ActionItemsProps): ReactElement {
  const [menu, setMenu] = useState<Menu | null>(null);
  const off = props.offHome;
  const empty =
    off === null
      ? "No open action items."
      : offHomeText(off, "action items", `Action items are kept on ${off.home}.`);
  return (
    <Widget id="dash-actions" title={title(props.actions)}>
      {props.actions.length === 0 ? (
        <p className="dash-empty">{empty}</p>
      ) : (
        <ul className="dash-rows">
          {props.actions.map((a) => (
            <ActionRow key={a.id} {...props} action={a} onMenu={setMenu} />
          ))}
        </ul>
      )}
      {menu === null ? null : (
        <RowContextMenu
          x={menu.x}
          y={menu.y}
          align="end"
          onClose={() => setMenu(null)}
          items={[
            { label: "Mark done", onSelect: () => props.onDone(menu.action.id) },
            ...(menu.action.item_id === null
              ? [{ label: "Promote to item", onSelect: () => props.onPromote(menu.action.id) }]
              : []),
            { label: "Drop", danger: true, onSelect: () => props.onDrop(menu.action.id) },
          ]}
        />
      )}
    </Widget>
  );
}
