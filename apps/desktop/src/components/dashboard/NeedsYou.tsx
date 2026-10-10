// Widget 1, Needs you (H-018 §2.1, H-112): only what waits on the owner,
// most urgent first, each row with one primary action. Releases open their
// review in a drawer, decisions open in Decisions, items open their drawer
// (U4). Rulings a bot recorded for the owner are one row, reviewed in a
// dialog and confirmed together (UX-016). WIP overrides are the lead's
// call, so they sit below, folded away, each opening its item too. Off the
// board's home, the home's rows say what to do there.

import { useCallback, useId, useState } from "react";
import type { MouseEvent, ReactElement } from "react";
import type { NeedsYou as Row, WipOverride } from "../../protocol/dashboard";
import type { Release } from "../../protocol/releases";
import CardLink from "../cards/CardLink";
import LinkedText from "../cards/LinkedText";
import { plural, releaseTitle, testLabel } from "../releases/labels";
import { names, when } from "./needsYouText";
import RelayedDialog from "./RelayedDialog";
import type { ConfirmOutcome } from "./useConfirmRelayed";
import { attentionRow } from "./attentionRows";
import type { AttentionActions } from "./attentionRows";
import { elsewhereKind, elsewhereRow } from "./elsewhereRows";

interface NeedsYouActions extends AttentionActions {
  readonly onReview: (release: Release) => void;
  readonly onDecision: (decisionId: string) => void;
  /** Confirms exactly these relayed rulings, as the dialog listed them. */
  readonly onConfirmRelayed: (decisionIds: readonly string[]) => Promise<ConfirmOutcome>;
  /** Opens the item's drawer (U4); `opener` takes focus back on close. */
  readonly onItem: (itemId: string, opener: HTMLElement) => void;
}

interface NeedsYouProps extends NeedsYouActions {
  readonly projectName: string;
  readonly rows: readonly Row[];
  /** The daemon's count, as the projects home counts (H-161); absent before 0.17. */
  readonly count?: number;
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
type Relayed = Extract<Shown, { readonly kind: "relayed" }>;

/** Releases, then decisions and relayed rulings, then P0s (§2.1). */
const ORDER: Readonly<Record<Shown["kind"], number>> = {
  serving_off: -1,
  release: 0,
  decision: 1,
  relayed: 2,
  owner_action: 1,
  permission_prompt: 1,
  p0: 3,
  owner_question: 4,
  // Weighed as an ordinary decision by the daemon, a bot stopped on its
  // terminal's permission prompt included (H-172).
  bot_waiting: 1,
  off_board: 6,
  // Your review comes after the bots' and is ranked like a release ruling (§4.3).
  pr_review: 1,
  pr_merge_stuck: 2,
  main_moved_outside: 0,
  // Under 20 GB free stops builds for every bot; a held cleanup can wait.
  disk_low: 2,
  cleanup_held: 6,
  routines_without_card: 7,
  // Sorted by the kind it carries (`orderOf`); this is the fallback.
  elsewhere: 5,
};

/** A row's place; another computer's row sorts as the kind it carries. */
function orderOf(r: Shown): number {
  if (r.kind !== "elsewhere") {
    return ORDER[r.kind];
  }
  const inner = elsewhereKind(r.row);
  return inner in ORDER ? ORDER[inner as Shown["kind"]] : ORDER.elsewhere;
}

/** "bot:<id>", "user", "device:…": who made a move, in words. */
function actorName(actor: string, botName: (id: string) => string): string {
  if (actor.startsWith("bot:")) {
    return botName(actor.slice(4));
  }
  return "You";
}

export interface RowViewProps {
  readonly glyph: string;
  readonly tone?: "bad" | "you";
  readonly title: ReactElement | string;
  readonly meta: string;
  /** None for a row that only informs: its fix is outside the app. */
  readonly action?: string;
  /** Starts with `action`, then says what it acts on (UX-010). */
  readonly label?: string;
  readonly onAction?: (event: MouseEvent<HTMLButtonElement>) => void;
  readonly disabled?: boolean;
  /** Why the action is unavailable, as its tooltip. */
  readonly why?: string;
  /** What to do on the computer that holds it: "Review", "Answer", "Confirm". */
  readonly verb: string;
  /** Off-home: the computer to act on it, in place of the button. */
  readonly elsewhere?: string;
}

function RowView(props: RowViewProps): ReactElement {
  const there = useId();
  const away = props.elsewhere !== undefined;
  return (
    <li className="dash-row" aria-describedby={away ? there : undefined}>
      <span className={`dash-glyph dash-tone-${props.tone ?? "plain"}`} aria-hidden="true">
        {props.glyph}
      </span>
      <div className="dash-row-text">
        <div className="dash-row-title">
          {typeof props.title === "string" ? <LinkedText text={props.title} /> : props.title}
        </div>
        <div className="dash-row-meta">
          <LinkedText text={props.meta} />
        </div>
      </div>
      {away ? (
        <span id={there} className="dash-elsewhere">
          {props.verb} on {props.elsewhere}
        </span>
      ) : props.action === undefined ? null : (
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
    verb: "Review",
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
    verb: action,
  };
}

function relayedRow(r: Relayed, props: NeedsYouProps, onReview: () => void): RowViewProps {
  const by = names(r.by.map((b) => props.botName(b.bot_id)));
  const count = plural(r.count, "ruling");
  return {
    glyph: "◇",
    title: `Confirm ${count} ${by} recorded for you`,
    meta: "Recorded on your behalf. Confirming makes them your own word.",
    action: `Review ${count}…`,
    label: `Review ${count} ${by} recorded for you`,
    onAction: onReview,
    disabled: !props.canApprove || props.confirming,
    why: props.canApprove ? undefined : "Only the owner can confirm rulings",
    verb: "Confirm",
  };
}

function p0Row(r: Extract<Shown, { readonly kind: "p0" }>, props: NeedsYouProps): RowViewProps {
  return {
    glyph: "‼",
    tone: "bad",
    title: (
      <>
        P0 · <CardLink id={r.id} /> · {r.title}
      </>
    ),
    meta: [
      props.columnName(r.column_key),
      r.assignee ? props.botName(r.assignee) : "Unassigned",
    ].join(" · "),
    action: "Open item",
    label: `Open item ${r.id}`,
    onAction: (event) => props.onItem(r.id, event.currentTarget),
    verb: "Open",
  };
}

/** No build can be published until hermesd.toml is fixed (H-100). */
function servingRow(r: Extract<Shown, { readonly kind: "serving_off" }>): RowViewProps {
  return { glyph: "⚠", tone: "bad", title: r.title, meta: r.reason, verb: "Fix it" };
}

/** Routines whose runs name no card (H-135 G5), listed once. */
function cardlessRow(
  r: Extract<Shown, { readonly kind: "routines_without_card" }>,
  props: NeedsYouProps,
): RowViewProps {
  return {
    glyph: "↻",
    title: `${plural(r.routines.length, "routine")} with no card on the board`,
    meta: r.routines.map((x) => `${x.name} (${props.botName(x.bot_id)})`).join(" · "),
    verb: "Check",
  };
}

function rowProps(
  r: Shown,
  props: NeedsYouProps,
  onReviewRelayed: () => void,
): RowViewProps | undefined {
  switch (r.kind) {
    case "serving_off":
      return servingRow(r);
    case "release":
      return releaseRow(r, props);
    case "decision":
      return decisionRow(r, props);
    case "relayed":
      return relayedRow(r, props, onReviewRelayed);
    case "p0":
      return p0Row(r, props);
    case "routines_without_card":
      return cardlessRow(r, props);
    case "elsewhere":
      return elsewhereRow(r.row, props);
    default:
      return attentionRow(r, props);
  }
}

function rowKey(r: Shown): string {
  switch (r.kind) {
    case "release":
      return `release-${r.release.id}`;
    case "relayed":
      return `relayed-${r.elsewhere ?? "here"}`;
    case "serving_off":
      return `serving-${r.elsewhere ?? "here"}`;
    case "routines_without_card":
      return `cardless-${r.elsewhere ?? "here"}`;
    case "elsewhere":
      return `elsewhere-${r.elsewhere}-${r.row.id}`;
    default:
      return `${r.kind}-${r.id}`;
  }
}

/** The lead's WIP overrides this week, folded under Needs you. */
function Overrides(props: {
  readonly overrides: readonly WipOverride[];
  readonly botName: (id: string) => string;
  readonly columnName: (key: string) => string;
  readonly onItem: (itemId: string, opener: HTMLElement) => void;
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
            <span>
              {props.columnName(o.column_key)} · <CardLink id={o.id} /> · {o.title} —{" "}
              <LinkedText text={o.note} /> — {actorName(o.actor, props.botName)} · {when(o.at)}
            </span>
            {/* Each override opens its item, as the P0 row does (UX-016 follow-up 2). */}
            <button
              type="button"
              className="btn btn-small"
              aria-label={`Open item ${o.id}`}
              onClick={(event) => props.onItem(o.id, event.currentTarget)}
            >
              Open item
            </button>
          </li>
        ))}
      </ul>
    </details>
  );
}

/** The relayed rulings this computer can confirm, as the daemon last read them. */
function localRelayed(rows: readonly Shown[]): Relayed | undefined {
  return rows.find((r): r is Relayed => r.kind === "relayed" && r.elsewhere === undefined);
}

export default function NeedsYou(props: NeedsYouProps): ReactElement {
  const [reviewing, setReviewing] = useState(false);
  // A daemon before 0.16.2 sends overrides as rows: they move below too.
  const legacy = props.rows.filter((r): r is Legacy => r.kind === "wip_override");
  // Owner actions have their own widget, "Commands for you to run", with Run.
  const rows = props.rows.filter(
    (r): r is Shown => r.kind !== "wip_override" && r.kind !== "owner_action",
  );
  // oxlint-disable-next-line unicorn/no-array-sort
  rows.sort((a, b) => orderOf(a) - orderOf(b));
  const relayed = localRelayed(rows);
  // Every ruling confirmed or answered meanwhile: nothing left to review,
  // and a later relay doesn't reopen the dialog by itself.
  if (reviewing && relayed === undefined) {
    setReviewing(false);
  }
  const openReview = useCallback((): void => setReviewing(true), []);
  const closeReview = useCallback((): void => setReviewing(false), []);
  const views = rows.flatMap((r) => {
    const view = rowProps(r, props, openReview);
    return view === undefined ? [] : [{ key: rowKey(r), view, elsewhere: r.elsewhere }];
  });
  // The number the projects home shows (H-161): an info row such as
  // routines with no card is listed, not counted. An older daemon sends
  // none, so its listed rows are counted as before.
  const count = props.count ?? views.length;
  return (
    <section className="dash-widget dash-wide" aria-labelledby="dash-needs-you">
      {/* At zero the empty sentence says it; no "· 0" badge (UX-010). */}
      <h2 id="dash-needs-you">Needs you{count > 0 ? ` · ${count}` : ""}</h2>
      {props.note ? (
        <p className="dash-note">
          <strong>{props.note}</strong>
        </p>
      ) : null}
      {views.length > 0 ? (
        <ul className="dash-rows">
          {views.map(({ key, view, elsewhere }) => (
            <RowView key={key} {...view} elsewhere={elsewhere} />
          ))}
        </ul>
      ) : null}
      {/* With the home away, nothing here can't be known (UX-016 §3). */}
      {rows.length === 0 && !props.note ? (
        <p className="dash-empty">Nothing needs you in {props.projectName}.</p>
      ) : null}
      <Overrides
        overrides={[...props.overrides, ...legacy]}
        botName={props.botName}
        columnName={props.columnName}
        onItem={props.onItem}
      />
      {reviewing && relayed ? (
        <RelayedDialog
          rulings={relayed.rulings}
          botName={props.botName}
          confirming={props.confirming}
          onConfirm={props.onConfirmRelayed}
          onOpen={props.onDecision}
          onClose={closeReview}
        />
      ) : null}
    </section>
  );
}
