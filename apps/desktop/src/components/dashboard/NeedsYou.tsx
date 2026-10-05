// Widget 1, Needs you (H-018 §2.1, H-112): only what waits on the owner,
// most urgent first, each row with one primary action. Releases open their
// review in a drawer, decisions open in Decisions, items open the board.
// Rulings a bot recorded for the owner are one row, confirmed together.
// WIP overrides are the lead's call, so they sit below, folded away. Off the
// board's home, the home's rows say where to act on them.

import type { ReactElement } from "react";
import type { NeedsYou as Row, WipOverride } from "../../protocol/dashboard";
import type { Release } from "../../protocol/releases";
import { releaseTitle, testLabel } from "../releases/labels";

interface NeedsYouActions {
  readonly onReview: (release: Release) => void;
  readonly onDecision: (decisionId: string) => void;
  readonly onBoard: () => void;
  readonly onConfirmRelayed: () => void;
}

interface NeedsYouProps extends NeedsYouActions {
  readonly projectName: string;
  readonly rows: readonly Row[];
  readonly overrides: readonly WipOverride[];
  /** Off-home, why the home's rows are missing. */
  readonly note?: string | null;
  /** Confirming a ruling is the owner's (the approve grant). */
  readonly canApprove: boolean;
  /** A bulk confirm is on its way. */
  readonly confirming: boolean;
  readonly botName: (id: string) => string;
  readonly columnName: (key: string) => string;
}

type Legacy = Extract<Row, { readonly kind: "wip_override" }>;
type Shown = Exclude<Row, Legacy>;

/** Releases, then decisions and relayed rulings, then P0s (§2.1). */
const ORDER: Readonly<Record<Shown["kind"], number>> = {
  release: 0,
  decision: 1,
  relayed: 2,
  p0: 3,
};

function plural(n: number, word: string): string {
  return `${n} ${word}${n === 1 ? "" : "s"}`;
}

function when(at: string): string {
  return new Date(at).toLocaleString([], {
    weekday: "short",
    hour: "2-digit",
    minute: "2-digit",
  });
}

/** "bot:<id>", "user", "device:…": who made a move, in words. */
function actorName(actor: string, botName: (id: string) => string): string {
  if (actor.startsWith("bot:")) {
    return botName(actor.slice(4));
  }
  return "You";
}

/** "Team Lead", "Team Lead and PM", "Team Lead, PM and QA". */
function names(list: readonly string[]): string {
  if (list.length <= 1) {
    return list[0] ?? "A bot";
  }
  return `${list.slice(0, -1).join(", ")} and ${list.at(-1) ?? ""}`;
}

interface RowViewProps {
  readonly glyph: string;
  readonly tone?: "bad" | "you";
  readonly title: ReactElement | string;
  readonly meta: string;
  readonly action: string;
  /** Starts with `action`, then says what it acts on (UX-010). */
  readonly label: string;
  readonly onAction: () => void;
  readonly disabled?: boolean;
  /** Why the action is unavailable, as its tooltip. */
  readonly why?: string;
  /** Off-home: the computer to act on it, in place of the button. */
  readonly elsewhere?: string;
}

function RowView(props: RowViewProps): ReactElement {
  return (
    <li className="dash-row">
      <span className={`dash-glyph dash-tone-${props.tone ?? "plain"}`} aria-hidden="true">
        {props.glyph}
      </span>
      <div className="dash-row-text">
        <div className="dash-row-title">{props.title}</div>
        <div className="dash-row-meta">{props.meta}</div>
      </div>
      {props.elsewhere === undefined ? (
        <button
          type="button"
          className="btn btn-small"
          aria-label={props.label}
          title={props.why}
          disabled={props.disabled}
          onClick={props.onAction}
        >
          {props.action}
        </button>
      ) : (
        <span className="dash-elsewhere">On {props.elsewhere}</span>
      )}
    </li>
  );
}

function releaseRow(
  r: Extract<Shown, { readonly kind: "release" }>,
  props: NeedsYouProps,
): RowViewProps {
  const tests = r.release.tests
    .map((t) => `${t.machine} ${testLabel(t.result).glyph} ${testLabel(t.result).word}`)
    .join(" · ");
  return {
    glyph: "▣",
    tone: "you",
    title: `${releaseTitle(r.release)} is ready for you to test · ${plural(r.release.items.length, "item")}`,
    meta: tests || "No test results yet",
    action: "Review",
    label: `Review ${releaseTitle(r.release)}`,
    onAction: () => props.onReview(r.release),
  };
}

function decisionRow(
  r: Extract<Shown, { readonly kind: "decision" }>,
  props: NeedsYouProps,
): RowViewProps {
  const action = r.relayed ? "Confirm" : "Answer";
  const due = r.deadline_at ? `Answer by ${when(r.deadline_at)}` : "Waiting for your answer";
  let meta = due;
  if (r.relayed) {
    meta = "A bot relayed your ruling: confirm it's yours";
  } else if (r.raised_by) {
    meta = `Raised by ${props.botName(r.raised_by)} · ${due}`;
  }
  return {
    glyph: "◆",
    title: r.title,
    meta,
    action,
    label: `${action}: ${r.title}`,
    onAction: () => props.onDecision(r.id),
  };
}

function relayedRow(
  r: Extract<Shown, { readonly kind: "relayed" }>,
  props: NeedsYouProps,
): RowViewProps {
  const by = names(r.by.map((b) => props.botName(b.bot_id)));
  return {
    glyph: "◇",
    title: `Confirm ${plural(r.count, "ruling")} ${by} recorded for you`,
    meta: "Recorded on your behalf. Confirming makes them your own word.",
    action: props.confirming ? "Confirming…" : "Confirm all",
    label: `Confirm all ${plural(r.count, "ruling")} ${by} recorded for you`,
    onAction: props.onConfirmRelayed,
    disabled: !props.canApprove || props.confirming,
    why: props.canApprove ? undefined : "Only the owner can confirm rulings",
  };
}

function p0Row(r: Extract<Shown, { readonly kind: "p0" }>, props: NeedsYouProps): RowViewProps {
  return {
    glyph: "‼",
    tone: "bad",
    title: (
      <>
        P0 · <span className="mono">{r.id}</span> {r.title}
      </>
    ),
    meta: [
      props.columnName(r.column_key),
      r.assignee ? props.botName(r.assignee) : "Unassigned",
    ].join(" · "),
    action: "Open board",
    label: `Open board at ${r.id}`,
    onAction: props.onBoard,
  };
}

function rowProps(r: Shown, props: NeedsYouProps): RowViewProps {
  switch (r.kind) {
    case "release":
      return releaseRow(r, props);
    case "decision":
      return decisionRow(r, props);
    case "relayed":
      return relayedRow(r, props);
    case "p0":
      return p0Row(r, props);
  }
}

function rowKey(r: Shown): string {
  switch (r.kind) {
    case "release":
      return `release-${r.release.id}`;
    case "relayed":
      return "relayed";
    default:
      return `${r.kind}-${r.id}`;
  }
}

/** The lead's WIP overrides this week, folded under Needs you. */
function Overrides(props: {
  readonly overrides: readonly WipOverride[];
  readonly botName: (id: string) => string;
  readonly columnName: (key: string) => string;
}): ReactElement | null {
  if (props.overrides.length === 0) {
    return null;
  }
  return (
    <details className="dash-info">
      <summary>{plural(props.overrides.length, "WIP override")} this week</summary>
      <ul>
        {props.overrides.map((o) => (
          <li key={`${o.id}-${o.at}`}>
            {props.columnName(o.column_key)} · <span className="mono">{o.id}</span> {o.title} —{" "}
            {o.note} — {actorName(o.actor, props.botName)} · {when(o.at)}
          </li>
        ))}
      </ul>
    </details>
  );
}

export default function NeedsYou(props: NeedsYouProps): ReactElement {
  // A daemon before 0.16.2 sends overrides as rows: they move below too.
  const legacy = props.rows.filter((r): r is Legacy => r.kind === "wip_override");
  const rows = props.rows.filter((r): r is Shown => r.kind !== "wip_override");
  // oxlint-disable-next-line unicorn/no-array-sort
  rows.sort((a, b) => ORDER[a.kind] - ORDER[b.kind]);
  return (
    <section className="dash-widget dash-wide" aria-labelledby="dash-needs-you">
      {/* At zero the empty sentence says it; no "· 0" badge (UX-010). */}
      <h2 id="dash-needs-you">Needs you{rows.length > 0 ? ` · ${rows.length}` : ""}</h2>
      {props.note ? <p className="dash-note">{props.note}</p> : null}
      {rows.length === 0 ? (
        <p className="dash-empty">Nothing needs you in {props.projectName}.</p>
      ) : (
        <ul className="dash-rows">
          {rows.map((r) => (
            <RowView key={rowKey(r)} {...rowProps(r, props)} elsewhere={r.elsewhere} />
          ))}
        </ul>
      )}
      <Overrides
        overrides={[...props.overrides, ...legacy]}
        botName={props.botName}
        columnName={props.columnName}
      />
    </section>
  );
}
