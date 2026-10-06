import type { ReactElement } from "react";
import type { ProjectRow, Source } from "../../protocol/gen/hermes/home/v1/home_pb";
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
  /** "#1", "#2"… among the projects that need the owner; undefined when nothing does. */
  readonly place: number | undefined;
  readonly now: number;
  /** Pinning needs the control grant. */
  readonly canPin: boolean;
  readonly onOpen: (projectId: string) => void;
  readonly onPin: (projectId: string, pinned: boolean) => void;
}

function Rank({ place }: { readonly place: number | undefined }): ReactElement {
  return place === undefined ? (
    <span className="home-rank home-rank-none" aria-label="Nothing needs you">
      —
    </span>
  ) : (
    <span className="home-rank">#{place}</span>
  );
}

/** The lead's latest summary, quoted, else when anything last happened. */
function Latest({ row, now }: { readonly row: ProjectRow; readonly now: number }): ReactElement {
  const summary = row.latestSummary;
  if (summary !== undefined && summary.text.length > 0) {
    return (
      <blockquote className="home-quote">
        {summary.at === undefined ? null : <b>{clock(summary.at, now)}: </b>}“{summary.text}”
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
    </dl>
  );
}

function Doing({ row }: { readonly row: ProjectRow }): ReactElement | null {
  if (row.doing.length === 0) {
    return null;
  }
  return (
    <ul className="home-doing" aria-label="Doing now">
      {row.doing.map((item) => (
        <li key={item.itemId}>
          <span className="home-doing-title">{item.title}</span>
          {item.assigneeName.length > 0 ? (
            <span className="home-doing-who">{item.assigneeName}</span>
          ) : null}
        </li>
      ))}
    </ul>
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
  readonly first: boolean;
  readonly now: number;
}): ReactElement {
  const { row, first, now } = props;
  if (row.legacy) {
    return <p className="home-why home-why-dim">Update needed for full info</p>;
  }
  const needs = (row.attention?.count ?? 0) > 0;
  const why = whyLine(row.attention);
  return (
    <>
      <p className={`home-why ${needs ? "home-why-needs" : "home-why-dim"}`}>
        {first ? `Needs you most: ${why}` : why}
      </p>
      <Facts row={row} />
      <Doing row={row} />
      <Latest row={row} now={now} />
    </>
  );
}

/** One project on the home (UX-024 §3): why it ranks, its release, its team, its latest news. */
export default function ProjectCard(props: ProjectCardProps): ReactElement {
  const { row, place, now } = props;
  const notes = staleNotes(row, props.sources, now);
  const first = place === 1;
  return (
    <article
      className={`home-card${first ? " home-card-first" : ""}`}
      aria-label={row.name}
      data-testid="project-card"
    >
      <header className="home-card-head">
        <Rank place={place} />
        <h3>{row.name}</h3>
        {props.canPin && !row.legacy ? <PinButton row={row} onPin={props.onPin} /> : null}
      </header>
      <Body row={row} first={first} now={now} />
      {notes.map((note) => (
        <p key={note} className="home-stale">
          ⚠ {note}
        </p>
      ))}
      <button
        type="button"
        className={`btn btn-small${first ? " btn-primary" : ""} home-open`}
        onClick={() => {
          props.onOpen(row.projectId);
        }}
      >
        Open project
      </button>
    </article>
  );
}
