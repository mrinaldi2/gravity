import type { ReactElement } from "react";
import type { ProjectRow, Source } from "../../protocol/gen/hermes/home/v1/home_pb";
import CardLink from "../cards/CardLink";
import {
  botsLine,
  clock,
  lastActivity,
  releasePill,
  runningLine,
  staleNotes,
  whyLine,
} from "./homeText";

interface ProjectCardProps {
  readonly row: ProjectRow;
  readonly sources: readonly Source[];
  /** "#1", "#2"… among the unpinned projects that need the owner; undefined otherwise. */
  readonly place: number | undefined;
  /** The project that needs the owner most, pinned or not: outlined. */
  readonly top: boolean;
  /** Who wrote the latest summary: the project's lead, else "Lead". */
  readonly leadName: string;
  readonly now: number;
  /** Pinning needs the control grant. */
  readonly canPin: boolean;
  readonly onOpen: (projectId: string) => void;
  readonly onPin: (projectId: string, pinned: boolean) => void;
}

function Rank(props: {
  readonly pinned: boolean;
  readonly place: number | undefined;
}): ReactElement {
  if (props.pinned) {
    return (
      <span className="home-rank home-rank-pin" aria-label="Pinned">
        📌
      </span>
    );
  }
  return props.place === undefined ? (
    <span className="home-rank home-rank-none" aria-label="Nothing needs you">
      —
    </span>
  ) : (
    <span className="home-rank">#{props.place}</span>
  );
}

/** The lead's latest summary, quoted and signed, else when anything last happened. */
function Latest(props: {
  readonly row: ProjectRow;
  readonly leadName: string;
  readonly now: number;
}): ReactElement {
  const { row, now } = props;
  const summary = row.latestSummary;
  if (summary !== undefined && summary.text.length > 0) {
    const who =
      summary.at === undefined ? props.leadName : `${props.leadName} · ${clock(summary.at, now)}`;
    return (
      <blockquote className="home-quote">
        <b>{who}: </b>“{summary.text}”
      </blockquote>
    );
  }
  const activity = lastActivity(row, now);
  return <p className="home-quote home-quote-dim">{activity ?? "No activity yet"}</p>;
}

function Facts({ row }: { readonly row: ProjectRow }): ReactElement {
  const pill = releasePill(row.currentRelease);
  return (
    <dl className="home-facts">
      <dt>Release</dt>
      <dd>
        <span className={`release-pill release-tone-${pill.tone}`}>{pill.text}</span>
      </dd>
      <dt>Running</dt>
      <dd>{runningLine(row)}</dd>
      <dt>Bots</dt>
      <dd>{botsLine(row)}</dd>
      {row.doing.length > 0 ? (
        <>
          <dt>Doing</dt>
          <dd>
            <ul className="home-doing">
              {row.doing.map((item) => (
                <li key={item.itemId}>
                  <CardLink id={item.itemId} /> ·{" "}
                  {item.assigneeName.length > 0
                    ? `${item.title} · ${item.assigneeName}`
                    : item.title}
                </li>
              ))}
            </ul>
          </dd>
        </>
      ) : null}
    </dl>
  );
}

function PinButton({
  row,
  onPin,
}: {
  readonly row: ProjectRow;
  readonly onPin: (projectId: string, pinned: boolean) => void;
}): ReactElement {
  return (
    <button
      type="button"
      className={`home-pin${row.pinned ? " home-pin-on" : ""}`}
      aria-pressed={row.pinned}
      aria-label={row.pinned ? `Unpin ${row.name}` : `Pin ${row.name} to the top`}
      title={row.pinned ? "Unpin: rank by need again" : "Pin to the top"}
      onClick={() => {
        onPin(row.projectId, !row.pinned);
      }}
    >
      {row.pinned ? "📌 Pinned" : "Pin"}
    </button>
  );
}

/** Why the card ranks where it does, then its facts and news; an older service has neither. */
function Body(props: {
  readonly row: ProjectRow;
  readonly top: boolean;
  readonly leadName: string;
  readonly now: number;
}): ReactElement {
  const { row, top, now } = props;
  if (row.legacy) {
    return <p className="home-why home-why-dim">Update needed for full info</p>;
  }
  const needs = (row.attention?.count ?? 0) > 0;
  const why = whyLine(row.attention);
  return (
    <>
      <p className={`home-why ${needs ? "home-why-needs" : "home-why-dim"}`}>
        {top ? `Needs you most: ${why}` : why}
      </p>
      <Facts row={row} />
      <Latest row={row} leadName={props.leadName} now={now} />
    </>
  );
}

/**
 * One project on the home (UX-024 §3): why it ranks, its release, its team, its
 * latest news. The whole card opens the project: "Open project" stretches over
 * it, and stays the keyboard target.
 */
export default function ProjectCard(props: ProjectCardProps): ReactElement {
  const { row, top, now } = props;
  const notes = staleNotes(row, props.sources, now);
  return (
    <article
      className={`home-card${top ? " home-card-first" : ""}`}
      aria-label={row.name}
      data-testid="project-card"
    >
      <header className="home-card-head">
        <Rank pinned={row.pinned} place={props.place} />
        <h3>{row.name}</h3>
        {props.canPin && !row.legacy ? <PinButton row={row} onPin={props.onPin} /> : null}
      </header>
      <Body row={row} top={top} leadName={props.leadName} now={now} />
      {notes.map((note) => (
        <p key={note} className="home-stale">
          ⚠ {note}
        </p>
      ))}
      <button
        type="button"
        className={`btn btn-small${top ? " btn-primary" : ""} home-open`}
        onClick={() => {
          props.onOpen(row.projectId);
        }}
      >
        Open project
      </button>
    </article>
  );
}
