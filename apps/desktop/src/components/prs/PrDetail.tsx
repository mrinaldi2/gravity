// One pull request (UX-051 "overview", "files", "checks", "review"): its card
// and title, who wants to merge what, and its Overview, Files and Checks;
// your review (Approve or Ask for changes), line comments, the 10 s Undo and
// the Re-check of what changed since you approved.
// Opening it moves focus to its heading (UX-013); the caller returns it on Back.

import { useEffect, useRef, useState } from "react";
import type { ReactElement } from "react";
import type { PullRequest, ReviewSettings } from "../../protocol/gen/hermes/pr/v1/pr_pb";
import { BlockerKind, PrState } from "../../protocol/gen/hermes/pr/v1/pr_pb";
import CardLink from "../cards/CardLink";
import { checksSummary, splitChecks } from "./checkText";
import MergeBar from "./MergeBar";
import { recheckBase, settingLine } from "./ownerText";
import { Chip } from "./PrList";
import PrChecks from "./PrChecks";
import PrFiles from "./PrFiles";
import type { SettingNote } from "./PrOverview";
import PrOverview from "./PrOverview";
import type { Tone } from "./prText";
import { age, branchMarker, sha7, waitsForYou } from "./prText";
import ReviewDialog from "./ReviewDialog";
import type { PrReview } from "./usePrReview";
import { usePrReview } from "./usePrReview";
import type { PrClient } from "./usePrOwner";

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

function Header(props: {
  readonly pr: PullRequest;
  readonly projectName: string;
  readonly onBack: () => void;
}): ReactElement {
  const { pr } = props;
  const pill = headPill(pr);
  // Opening a PR moves focus to its heading (UX-013).
  const heading = useRef<HTMLHeadingElement>(null);
  useEffect(() => {
    heading.current?.focus();
  }, []);
  return (
    <header className="pr-detail-head">
      <div className="pr-detail-title">
        <button type="button" className="pr-crumb" onClick={props.onBack}>
          {props.projectName} › Pull requests ›
        </button>
        <h3 tabIndex={-1} ref={heading}>
          #{pr.number} · {pr.itemId ? <CardLink id={pr.itemId} /> : null} {pr.itemTitle || pr.title}
        </h3>
      </div>
      {pill === null ? null : <Chip text={pill.text} tone={pill.tone} />}
    </header>
  );
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

function Sections(props: {
  readonly pr: PullRequest;
  readonly section: Section;
  readonly onSection: (section: Section) => void;
}): ReactElement {
  const { pr } = props;
  const head = splitChecks(pr).head;
  const sections: readonly { readonly key: Section; readonly word: string }[] = [
    { key: "overview", word: "Overview" },
    { key: "files", word: pr.filesChanged > 0 ? `Files · ${pr.filesChanged}` : "Files" },
    { key: "checks", word: `Checks · ${head.length}` },
  ];
  return (
    <div className="pr-sections" role="tablist" aria-label="Pull request">
      {sections.map(({ key, word }) => (
        <button
          key={key}
          type="button"
          role="tab"
          aria-selected={key === props.section}
          className={`pr-section ${key === props.section ? "pr-section-on" : ""}`}
          onClick={() => props.onSection(key)}
        >
          {word}
        </button>
      ))}
      <span className="pr-sections-sum pr-dim">{checksSummary(head).text}</span>
    </div>
  );
}

/** "Review…" on your row, or "Re-check…" when you approved an older change. */
function OwnerAction(props: {
  readonly pr: PullRequest;
  readonly review: PrReview;
  readonly onRecheck: (() => void) | null;
}): ReactElement {
  if (props.onRecheck !== null) {
    return (
      <button type="button" className="btn btn-small btn-primary" onClick={props.onRecheck}>
        Re-check…
      </button>
    );
  }
  return (
    <button
      type="button"
      className={`btn btn-small ${waitsForYou(props.pr) ? "btn-primary" : ""}`}
      onClick={props.review.openReview}
    >
      Review…
    </button>
  );
}

/**
 * Which section and delta show: a Re-check opens Files from your approval
 * (decision 11). `recheck` moves there; null when you approved no older change.
 */
function useOpening(pr: PullRequest, asRecheck: boolean | undefined) {
  const base = recheckBase(pr);
  const first = asRecheck === true && base !== null;
  const [section, setSection] = useState<Section>(first ? "files" : "overview");
  const [from, setFrom] = useState<string | null>(first ? base : null);
  const recheck =
    base === null
      ? null
      : (): void => {
          setFrom(base);
          setSection("files");
        };
  return { section, setSection, from, setFrom, base, recheck };
}

function settingNote(
  settings: ReviewSettings | null | undefined,
  onChange: (() => void) | undefined,
): SettingNote | null {
  return settings ? { line: settingLine(settings), onChange: onChange ?? null } : null;
}

export default function PrDetail(props: {
  readonly client: PrClient;
  readonly pr: PullRequest;
  readonly connected: boolean;
  readonly now: number;
  readonly projectName: string;
  readonly onBack: () => void;
  /** Reads the PR again after one of your writes. */
  readonly reload: () => void;
  readonly settings?: ReviewSettings | null;
  readonly onOpenSettings?: () => void;
  /** Opened from a Re-check row: show the delta since your approval (decision 11). */
  readonly recheck?: boolean;
  /** The Undo counts down by the second; stories pin the time instead. */
  readonly live?: boolean;
}): ReactElement {
  const { pr, now, client, connected } = props;
  const { section, setSection, from, setFrom, base, recheck } = useOpening(pr, props.recheck);
  const review = usePrReview(client, pr, connected, props.reload);

  return (
    <div className="pr-detail">
      <Header pr={pr} projectName={props.projectName} onBack={props.onBack} />
      <Byline pr={pr} now={now} />
      <MergeBar
        pr={pr}
        now={now}
        live={props.live ?? true}
        canUndo={review.canWrite}
        owner={review.owner}
      />
      <Sections pr={pr} section={section} onSection={setSection} />
      <div className="pr-detail-body" role="tabpanel">
        {section === "overview" ? (
          <PrOverview
            pr={pr}
            now={now}
            ownerAction={
              review.reviewable ? <OwnerAction pr={pr} review={review} onRecheck={recheck} /> : null
            }
            setting={settingNote(props.settings, props.onOpenSettings)}
          />
        ) : null}
        {section === "files" ? (
          <PrFiles
            client={client}
            pr={pr}
            connected={connected}
            from={from}
            onFrom={setFrom}
            recheck={base}
            onReview={review.reviewable ? review.openReview : null}
            comments={review.comments}
          />
        ) : null}
        {section === "checks" ? <PrChecks pr={pr} /> : null}
      </div>
      {review.reviewing === null ? null : (
        <ReviewDialog
          pr={pr}
          sha={review.reviewing}
          owner={review.owner}
          onClose={review.closeReview}
        />
      )}
    </div>
  );
}
