// One pull request (UX-051 "overview", "files", "checks"): its card and
// title, who wants to merge what, and its Overview, Files and Checks.

import { useState } from "react";
import type { ReactElement } from "react";
import type { PullRequest } from "../../protocol/gen/hermes/pr/v1/pr_pb";
import { BlockerKind, PrState } from "../../protocol/gen/hermes/pr/v1/pr_pb";
import type { PrApi } from "../../protocol/prs";
import CardLink from "../cards/CardLink";
import { checksSummary, splitChecks } from "./checkText";
import { Chip } from "./PrList";
import PrChecks from "./PrChecks";
import PrFiles from "./PrFiles";
import PrOverview from "./PrOverview";
import type { Tone } from "./prText";
import { age, branchMarker, sha7, waitsForYou } from "./prText";

type Section = "overview" | "files" | "checks";

/** The header's pill: what matters most about the PR right now. */
function headPill(pr: PullRequest): { readonly text: string; readonly tone: Tone } | null {
  if (pr.state === PrState.MERGED) {
    return { text: "✓ Merged", tone: "ok" };
  }
  if (pr.state === PrState.CLOSED) {
    return { text: "Closed", tone: "dim" };
  }
  const marker = branchMarker(pr);
  if (marker !== null) {
    return marker.tone === "warn" ? { text: "⚠ Has conflicts", tone: "warn" } : marker;
  }
  if (waitsForYou(pr)) {
    return { text: "▲ Waiting for your review", tone: "you" };
  }
  if (pr.mergeable?.blockers.some((b) => b.kind === BlockerKind.CHECK_PENDING)) {
    return { text: "◌ Checks running", tone: "dim" };
  }
  return null;
}

function Byline({ pr, now }: { readonly pr: PullRequest; readonly now: number }): ReactElement {
  const verb = pr.state === PrState.MERGED ? "merged" : "wants to merge";
  return (
    <div className="pr-byline">
      {pr.author?.name ?? "A bot"} {verb} <span className="pr-mono">{pr.branch}</span> into{" "}
      <span className="pr-mono">{pr.base}</span> · latest{" "}
      <span className="pr-sha">{sha7(pr.headSha)}</span>
      {pr.openedAt ? ` · opened ${age(pr.openedAt, now)} ago` : ""}
      {pr.movedUnreported ? " · ⟳ new commits since the last reported push" : ""}
    </div>
  );
}

export default function PrDetail(props: {
  readonly client: PrApi;
  readonly pr: PullRequest;
  readonly connected: boolean;
  readonly now: number;
  readonly projectName: string;
  readonly onBack: () => void;
}): ReactElement {
  const { pr, now } = props;
  const [section, setSection] = useState<Section>("overview");
  const pill = headPill(pr);
  const head = splitChecks(pr).head;
  const sections: readonly { readonly key: Section; readonly word: string }[] = [
    { key: "overview", word: "Overview" },
    { key: "files", word: pr.filesChanged > 0 ? `Files · ${pr.filesChanged}` : "Files" },
    { key: "checks", word: `Checks · ${head.length}` },
  ];
  return (
    <div className="pr-detail">
      <header className="pr-detail-head">
        <div className="pr-detail-title">
          <button type="button" className="pr-crumb" onClick={props.onBack}>
            {props.projectName} › Pull requests ›
          </button>
          <h3>
            #{pr.number} · {pr.itemId ? <CardLink id={pr.itemId} /> : null}{" "}
            {pr.itemTitle || pr.title}
          </h3>
        </div>
        {pill === null ? null : <Chip text={pill.text} tone={pill.tone} />}
      </header>
      <Byline pr={pr} now={now} />
      <div className="pr-sections" role="tablist" aria-label="Pull request">
        {sections.map(({ key, word }) => (
          <button
            key={key}
            type="button"
            role="tab"
            aria-selected={key === section}
            className={`pr-section ${key === section ? "pr-section-on" : ""}`}
            onClick={() => setSection(key)}
          >
            {word}
          </button>
        ))}
        <span className="pr-sections-sum pr-dim">{checksSummary(head).text}</span>
      </div>
      <div className="pr-detail-body" role="tabpanel">
        {section === "overview" ? <PrOverview pr={pr} now={now} /> : null}
        {section === "files" ? (
          <PrFiles client={props.client} pr={pr} connected={props.connected} />
        ) : null}
        {section === "checks" ? <PrChecks pr={pr} /> : null}
      </div>
    </div>
  );
}
