// Widget 1, Needs you (H-018 §2.1): what waits on the owner, most urgent
// first, each row with one primary action. Releases open their review in a
// drawer, decisions open in Decisions, items open the board.

import type { ReactElement } from "react";
import type { NeedsYou as Row } from "../../protocol/dashboard";
import type { Release } from "../../protocol/releases";
import { releaseTitle, testLabel } from "../releases/labels";

interface NeedsYouActions {
  readonly onReview: (release: Release) => void;
  readonly onDecision: (decisionId: string) => void;
  readonly onBoard: () => void;
}

interface NeedsYouProps extends NeedsYouActions {
  readonly projectName: string;
  readonly rows: readonly Row[];
  readonly botName: (id: string) => string;
  readonly columnName: (key: string) => string;
}

/** Releases, then decisions, then P0s, then WIP overrides (§2.1). */
const ORDER: Readonly<Record<Row["kind"], number>> = {
  release: 0,
  decision: 1,
  p0: 2,
  wip_override: 3,
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

function RowView(props: {
  readonly glyph: string;
  readonly tone?: "bad" | "you";
  readonly title: ReactElement | string;
  readonly meta: string;
  readonly action: string;
  /** Starts with `action`, then says what it acts on (UX-010). */
  readonly label: string;
  readonly onAction: () => void;
}): ReactElement {
  return (
    <li className="dash-row">
      <span className={`dash-glyph dash-tone-${props.tone ?? "plain"}`} aria-hidden="true">
        {props.glyph}
      </span>
      <div className="dash-row-text">
        <div className="dash-row-title">{props.title}</div>
        <div className="dash-row-meta">{props.meta}</div>
      </div>
      <button
        type="button"
        className="btn btn-small"
        aria-label={props.label}
        onClick={props.onAction}
      >
        {props.action}
      </button>
    </li>
  );
}

function row(r: Row, props: NeedsYouProps): ReactElement {
  switch (r.kind) {
    case "release": {
      const tests = r.release.tests
        .map((t) => `${t.machine} ${testLabel(t.result).glyph} ${testLabel(t.result).word}`)
        .join(" · ");
      return (
        <RowView
          key={`release-${r.release.id}`}
          glyph="▣"
          tone="you"
          title={`${releaseTitle(r.release)} is ready for you to test · ${plural(r.release.items.length, "item")}`}
          meta={tests || "No test results yet"}
          action="Review"
          label={`Review ${releaseTitle(r.release)}`}
          onAction={() => props.onReview(r.release)}
        />
      );
    }
    case "decision": {
      const action = r.relayed ? "Confirm" : "Answer";
      const due = r.deadline_at ? `Answer by ${when(r.deadline_at)}` : "Waiting for your answer";
      return (
        <RowView
          key={`decision-${r.id}`}
          glyph="◆"
          title={r.title}
          meta={
            r.relayed
              ? "A bot relayed your ruling: confirm it's yours"
              : r.raised_by
                ? `Raised by ${props.botName(r.raised_by)} · ${due}`
                : due
          }
          action={action}
          label={`${action}: ${r.title}`}
          onAction={() => props.onDecision(r.id)}
        />
      );
    }
    case "p0":
      return (
        <RowView
          key={`p0-${r.id}`}
          glyph="‼"
          tone="bad"
          title={
            <>
              P0 · <span className="mono">{r.id}</span> {r.title}
            </>
          }
          meta={[
            props.columnName(r.column_key),
            r.assignee ? props.botName(r.assignee) : "Unassigned",
          ].join(" · ")}
          action="Open board"
          label={`Open board at ${r.id}`}
          onAction={props.onBoard}
        />
      );
    case "wip_override":
      return (
        <RowView
          key={`wip-${r.id}-${r.at}`}
          glyph="⚑"
          title={
            <>
              WIP override · {props.columnName(r.column_key)} · <span className="mono">{r.id}</span>{" "}
              {r.title}
            </>
          }
          meta={`${r.note} — ${actorName(r.actor, props.botName)} · ${when(r.at)}`}
          action="Open board"
          label={`Open board at ${r.id}`}
          onAction={props.onBoard}
        />
      );
  }
}

export default function NeedsYou(props: NeedsYouProps): ReactElement {
  const rows = [...props.rows];
  // oxlint-disable-next-line unicorn/no-array-sort
  rows.sort((a, b) => ORDER[a.kind] - ORDER[b.kind]);
  return (
    <section className="dash-widget dash-wide" aria-labelledby="dash-needs-you">
      {/* At zero the empty sentence says it; no "· 0" badge (UX-010). */}
      <h2 id="dash-needs-you">Needs you{rows.length > 0 ? ` · ${rows.length}` : ""}</h2>
      {rows.length === 0 ? (
        <p className="dash-empty">Nothing needs you in {props.projectName}.</p>
      ) : (
        <ul className="dash-rows">{rows.map((r) => row(r, props))}</ul>
      )}
    </section>
  );
}
