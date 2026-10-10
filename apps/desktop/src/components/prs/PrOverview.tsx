// A PR's overview (UX-051 "overview", decisions 3, 4 and 11): what it waits
// for, each reviewer's verdict bound to a commit, the findings, the change
// note, its checks at the head, and the cleanup once merged.

import type { ReactElement } from "react";
import type { Finding, PullRequest, Review } from "../../protocol/gen/hermes/pr/v1/pr_pb";
import { PrState, Severity } from "../../protocol/gen/hermes/pr/v1/pr_pb";
import CardLink from "../cards/CardLink";
import LinkedText from "../cards/LinkedText";
import { cleanupItemLine, cleanupLine } from "./checkText";
import PrChecks from "./PrChecks";
import type { ReviewState } from "./prText";
import {
  age,
  otherBlockers,
  OWNER_ROLE,
  reviewOf,
  reviewRoles,
  reviewState,
  roleLabel,
  sha7,
  stateLine,
  waitingFor,
} from "./prText";

const MERGE_RULE = "Merges to main when every review is in and checks pass.";

/** The top line: what the PR waits for, or that it merged (decision 4). */
function MergeLine({ pr, now }: { readonly pr: PullRequest; readonly now: number }): ReactElement {
  const done = stateLine(pr, now);
  if (done !== null) {
    const tone = pr.state === PrState.MERGED ? "ok" : "dim";
    return <div className={`pr-banner pr-banner-${tone}`}>{done}</div>;
  }
  const waiting = waitingFor(pr);
  const others = otherBlockers(pr);
  return (
    <div className="pr-banner" role="status">
      <b>{MERGE_RULE}</b>{" "}
      {pr.mergeable?.ok === true ? (
        "Everything is in: it merges next."
      ) : waiting ? (
        <>
          Waiting for: <b>{waiting}</b>.
        </>
      ) : null}
      {others.length > 0 ? (
        <ul className="pr-banner-more">
          {others.map((b) => (
            <li key={`${b.kind}-${b.text}`}>
              <LinkedText text={b.text} />
              {b.paths.length > 0 ? <span className="pr-mono"> · {b.paths.join(", ")}</span> : null}
            </li>
          ))}
        </ul>
      ) : null}
    </div>
  );
}

function who(role: string, review: Review | undefined): string {
  const reviewer = review?.reviewer?.who;
  if (reviewer?.case === "owner") {
    return reviewer.value.deviceName ? `You · ${reviewer.value.deviceName}` : "You";
  }
  return reviewer?.case === "bot" ? reviewer.value.name : roleLabel(role);
}

/** The line under a reviewer's state: who and what they said, or why they haven't yet. */
function reviewMeta(pr: PullRequest, role: string, review: Review | undefined): string {
  if (review !== undefined) {
    return review.summary ? `${who(role, review)} · "${review.summary}"` : who(role, review);
  }
  const waiver = pr.waivers.find((w) => w.role === role);
  if (waiver !== undefined) {
    return `Not needed: ${waiver.reason} (${waiver.by?.name ?? "the lead"})`;
  }
  if (role !== OWNER_ROLE) {
    return "Asked to review";
  }
  const wait = pr.mergeable?.blockers.find((b) => b.subject === OWNER_ROLE)?.text;
  return wait ?? (pr.ownerFlagged ? `Flagged for you: ${pr.ownerFlagReason}` : "Your review");
}

/** "✓ Approved f0678fc", with the commit set as a sha. */
function StateText({ state }: { readonly state: ReviewState }): ReactElement {
  const [before, after] = state.text.split("{sha}");
  return (
    <div className={`pr-tone-${state.tone}`}>
      {state.glyph} {before}
      {after === undefined ? null : (
        <>
          <span className="pr-sha">{sha7(state.sha)}</span>
          {after}
        </>
      )}
    </div>
  );
}

function ReviewRow(props: {
  readonly pr: PullRequest;
  readonly role: string;
  readonly now: number;
}): ReactElement {
  const { pr, role } = props;
  const review = reviewOf(pr, role);
  return (
    <li className="pr-review">
      <span className="pr-review-role">{roleLabel(role)}</span>
      <div className="pr-review-what">
        <StateText state={reviewState(pr, role, review)} />
        <div className="pr-dim">{reviewMeta(pr, role, review)}</div>
      </div>
      <span className="pr-dim">{review === undefined ? "" : age(review.at, props.now)}</span>
    </li>
  );
}

const SEVERITY_WORDS: Readonly<Record<Severity, string>> = {
  [Severity.UNSPECIFIED]: "Finding",
  [Severity.MUST]: "Must-fix",
  [Severity.SHOULD]: "Should-fix",
  [Severity.NIT]: "Nit",
};

function FindingRow(props: { readonly finding: Finding; readonly by: string }): ReactElement {
  const { finding } = props;
  const resolved = finding.resolvedIn !== "";
  const where = finding.path ? `${finding.path}${finding.line > 0 ? `:${finding.line}` : ""}` : "";
  return (
    <li className="pr-finding">
      <span className={resolved ? "pr-tone-ok" : "pr-dim"} aria-hidden="true">
        {resolved ? "☑" : "☐"}
      </span>
      <div>
        <div>
          {SEVERITY_WORDS[finding.severity]} · <LinkedText text={finding.text} />
        </div>
        <div className="pr-dim">
          {props.by}
          {resolved ? ` · fixed in ${sha7(finding.resolvedIn)}` : " · open"}
          {finding.followUpItemId ? (
            <>
              {" · follow-up "}
              <CardLink id={finding.followUpItemId} />
            </>
          ) : null}
          {where ? <span className="pr-mono"> · {where}</span> : null}
        </div>
      </div>
    </li>
  );
}

function Findings({ pr }: { readonly pr: PullRequest }): ReactElement | null {
  const all = pr.reviews.flatMap((r) => r.findings.map((f) => ({ f, by: who(r.role, r) })));
  if (all.length === 0) {
    return null;
  }
  const resolved = all.filter(({ f }) => f.resolvedIn !== "").length;
  return (
    <section className="pr-box" aria-label="Findings">
      <h4>
        Findings · {resolved} of {all.length} resolved
      </h4>
      <ul className="pr-plain">
        {all.map(({ f, by }) => (
          <FindingRow key={`${by}-${f.text}`} finding={f} by={by} />
        ))}
      </ul>
    </section>
  );
}

function CleanupBox({ pr }: { readonly pr: PullRequest }): ReactElement | null {
  if (pr.state !== PrState.MERGED) {
    return null;
  }
  const line = cleanupLine(pr.cleanup);
  return (
    <section className="pr-box" aria-label="Cleanup">
      <h4>Cleanup</h4>
      <div className={`pr-tone-${line?.tone ?? "dim"}`}>{line?.text ?? "○ Not cleaned up yet"}</div>
      {pr.cleanup && pr.cleanup.items.length > 0 ? (
        <ul className="pr-plain pr-dim pr-cleanup-items">
          {pr.cleanup.items.map((item) => (
            <li key={item.jobId}>{cleanupItemLine(item)}</li>
          ))}
        </ul>
      ) : null}
    </section>
  );
}

export default function PrOverview(props: {
  readonly pr: PullRequest;
  readonly now: number;
}): ReactElement {
  const { pr, now } = props;
  return (
    <div className="pr-overview">
      <div className="pr-col">
        <MergeLine pr={pr} now={now} />
        <section className="pr-box" aria-label="Reviews">
          <h4>
            Reviews · at <span className="pr-sha">{sha7(pr.headSha)}</span>
          </h4>
          <ul className="pr-plain">
            {reviewRoles(pr).map((role) => (
              <ReviewRow key={role} pr={pr} role={role} now={now} />
            ))}
          </ul>
        </section>
        <Findings pr={pr} />
      </div>
      <div className="pr-col">
        <section className="pr-box" aria-label="Change note">
          <h4>Change note · {pr.author?.name ?? ""}</h4>
          <div className="pr-note">
            {pr.changeNote ? (
              <LinkedText text={pr.changeNote} />
            ) : (
              <span className="pr-dim">No change note yet.</span>
            )}
          </div>
        </section>
        <PrChecks pr={pr} compact />
        <CleanupBox pr={pr} />
      </div>
    </div>
  );
}
