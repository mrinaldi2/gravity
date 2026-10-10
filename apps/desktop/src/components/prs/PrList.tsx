// The Pull requests tab's list (UX-051 "list"): one row per PR, always with
// its card, its review chips and its checks, and a marker when it is behind
// main or has conflicts. Read-only: reviewing is H-277's.

import type { ReactElement } from "react";
import type { PullRequest } from "../../protocol/gen/hermes/pr/v1/pr_pb";
import { PrState } from "../../protocol/gen/hermes/pr/v1/pr_pb";
import CardLink from "../cards/CardLink";
import { checksSummary, splitChecks } from "./checkText";
import type { Tone } from "./prText";
import { age, branchMarker, clock, reviewChip, reviewRoles, waitsForYou } from "./prText";

export type PrFilter = "you" | "open" | "merged" | "closed";

const FILTERS: readonly { readonly key: PrFilter; readonly word: string }[] = [
  { key: "you", word: "Waiting for you" },
  { key: "open", word: "Open" },
  { key: "merged", word: "Merged" },
  { key: "closed", word: "Closed" },
];

export function inFilter(pr: PullRequest, filter: PrFilter): boolean {
  switch (filter) {
    case "you":
      return waitsForYou(pr);
    case "open":
      return pr.state === PrState.OPEN || pr.state === PrState.MERGING;
    case "merged":
      return pr.state === PrState.MERGED;
    case "closed":
      return pr.state === PrState.CLOSED;
  }
}

export function Chip(props: { readonly text: string; readonly tone: Tone }): ReactElement {
  return <span className={`pr-chip pr-tone-${props.tone}`}>{props.text}</span>;
}

/** "Desktop Dev · into main · 14 files +412 −37 · 3h", or when it merged. */
function meta(pr: PullRequest, now: number): string {
  const parts = [pr.author?.name ?? ""];
  if (pr.state === PrState.MERGED) {
    parts.push(`✓ merged into ${pr.base} ${clock(pr.mergedAt, now)}`);
    return parts.join(" · ");
  }
  parts.push(`into ${pr.base}`);
  if (pr.filesChanged > 0) {
    parts.push(`${pr.filesChanged} files +${pr.additions} −${pr.deletions}`);
  }
  if (pr.docsChanged) {
    parts.push("docs changed");
  }
  parts.push(age(pr.openedAt, now));
  return parts.filter(Boolean).join(" · ");
}

/** The right of a row: behind main or conflicts, merged, merging or closed. */
function Marker({ pr }: { readonly pr: PullRequest }): ReactElement | null {
  const marker = branchMarker(pr);
  if (marker !== null) {
    return <span className={`pr-marker pr-tone-${marker.tone}`}>{marker.text}</span>;
  }
  switch (pr.state) {
    case PrState.MERGED:
      return <Chip text="✓ Merged" tone="ok" />;
    case PrState.MERGING:
      return <Chip text="◌ Merging" tone="dim" />;
    case PrState.CLOSED:
      return <Chip text="Closed" tone="dim" />;
    default:
      return null;
  }
}

function Row(props: {
  readonly pr: PullRequest;
  readonly now: number;
  readonly onOpen: (number: number) => void;
}): ReactElement {
  const { pr, now } = props;
  const checks = checksSummary(splitChecks(pr).head);
  const title = pr.itemTitle || pr.title;
  return (
    <li className="pr-row">
      <span className="pr-row-number">#{pr.number}</span>
      <div className="pr-row-main">
        <div className="pr-row-title">
          {pr.itemId ? <CardLink id={pr.itemId} /> : null}{" "}
          <button
            type="button"
            className="pr-row-open"
            data-pr={pr.number}
            aria-label={`${title}, pull request #${pr.number}`}
            onClick={() => props.onOpen(pr.number)}
          >
            {title}
          </button>
        </div>
        <div className="pr-row-meta">{meta(pr, now)}</div>
        <div className="pr-chips">
          {reviewRoles(pr).map((role) => {
            const chip = reviewChip(pr, role);
            return <Chip key={role} text={chip.text} tone={chip.tone} />;
          })}
          <Chip text={checks.text} tone={checks.tone} />
        </div>
      </div>
      <Marker pr={pr} />
    </li>
  );
}

interface PrListProps {
  readonly prs: readonly PullRequest[];
  readonly filter: PrFilter;
  readonly now: number;
  readonly onFilter: (filter: PrFilter) => void;
  readonly onOpen: (number: number) => void;
}

export default function PrList(props: PrListProps): ReactElement {
  const { prs, filter } = props;
  const shown = prs.filter((pr) => inFilter(pr, filter));
  return (
    <div className="pr-list-pane">
      <div className="pr-filters" role="group" aria-label="Show">
        {FILTERS.map(({ key, word }) => (
          <button
            key={key}
            type="button"
            className={`pr-filter ${key === filter ? "pr-filter-on" : ""}`}
            aria-pressed={key === filter}
            onClick={() => props.onFilter(key)}
          >
            {word} · {prs.filter((pr) => inFilter(pr, key)).length}
          </button>
        ))}
      </div>
      {shown.length === 0 ? (
        <p className="pr-empty">No pull requests here.</p>
      ) : (
        <ul className="pr-rows" aria-label="Pull requests">
          {shown.map((pr) => (
            <Row key={pr.number} pr={pr} now={props.now} onOpen={props.onOpen} />
          ))}
        </ul>
      )}
    </div>
  );
}
