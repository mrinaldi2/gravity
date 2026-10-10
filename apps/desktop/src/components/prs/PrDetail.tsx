// One pull request (UX-051 "overview", "files", "checks", "review"): its card
// and title, who wants to merge what, and its Overview, Files and Checks;
// your review (Approve or Ask for changes), line comments, the 10 s Undo and
// the Re-check of what changed since you approved.
// Opening it moves focus to its heading (UX-013); the caller returns it on Back.

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { ReactElement } from "react";
import type { PullRequest, ReviewSettings } from "../../protocol/gen/hermes/pr/v1/pr_pb";
import { BlockerKind, PrState } from "../../protocol/gen/hermes/pr/v1/pr_pb";
import CardLink from "../cards/CardLink";
import { checksSummary, splitChecks } from "./checkText";
import type { CommentWrite, Thread } from "./LineComments";
import { lineKey, threadsOf } from "./LineComments";
import MergeBar from "./MergeBar";
import { recheckBase, settingLine } from "./ownerText";
import { Chip } from "./PrList";
import PrChecks from "./PrChecks";
import PrFiles from "./PrFiles";
import PrOverview from "./PrOverview";
import type { Tone } from "./prText";
import { age, branchMarker, sha7, waitsForYou } from "./prText";
import ReviewDialog from "./ReviewDialog";
import { usePrComments } from "./usePullRequests";
import type { PrClient } from "./usePrOwner";
import { useOwnerAct } from "./usePrOwner";

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

/** Threads by the line they sit under on the head, and those whose line is gone. */
function placeThreads(threads: readonly Thread[]): {
  readonly at: ReadonlyMap<string, readonly Thread[]>;
  readonly outdated: readonly Thread[];
} {
  const at = new Map<string, Thread[]>();
  const outdated: Thread[] = [];
  for (const thread of threads) {
    const { root } = thread;
    if (root.outdated) {
      outdated.push(thread);
      continue;
    }
    const key = lineKey(root.path, root.side, root.line);
    at.set(key, [...(at.get(key) ?? []), thread]);
  }
  return { at, outdated };
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
  const base = recheckBase(pr);
  const recheckFirst = props.recheck === true && base !== null;
  const [section, setSection] = useState<Section>(recheckFirst ? "files" : "overview");
  const [from, setFrom] = useState<string | null>(recheckFirst ? base : null);
  const [reviewing, setReviewing] = useState<string | null>(null);
  const opener = useRef<HTMLElement | null>(null);
  const heading = useRef<HTMLHeadingElement>(null);
  useEffect(() => {
    heading.current?.focus();
  }, []);
  const comments = usePrComments(client, pr, connected);
  const reloadComments = comments.reload;
  const { reload } = props;
  const after = useCallback(() => {
    reload();
    reloadComments();
  }, [reload, reloadComments]);
  const owner = useOwnerAct(client, after);
  const canWrite =
    connected &&
    client.hasGrant("approve") &&
    (pr.state === PrState.OPEN || pr.state === PrState.MERGING);
  const placed = useMemo(
    () => placeThreads(threadsOf(comments.data?.comments ?? [])),
    [comments.data],
  );

  const openReview = (event: { readonly currentTarget: HTMLElement }): void => {
    opener.current = event.currentTarget;
    owner.clear();
    setReviewing(pr.headSha);
  };
  const closeReview = useCallback((): void => {
    setReviewing(null);
    (opener.current?.isConnected === true ? opener.current : heading.current)?.focus();
  }, []);
  const recheck = (): void => {
    setFrom(base);
    setSection("files");
  };
  const onWrite = (write: CommentWrite): Promise<boolean> =>
    owner.act(
      write.type === "pr_comment_resolve"
        ? { ...write, project_id: pr.projectId, number: pr.number }
        : { ...write, project_id: pr.projectId, number: pr.number, sha: pr.headSha },
    );

  const reviewable = canWrite && pr.state === PrState.OPEN;
  let ownerAction: ReactElement | null = null;
  if (reviewable) {
    ownerAction =
      base === null ? (
        <button
          type="button"
          className={`btn btn-small ${waitsForYou(pr) ? "btn-primary" : ""}`}
          onClick={openReview}
        >
          Review…
        </button>
      ) : (
        <button type="button" className="btn btn-small btn-primary" onClick={recheck}>
          Re-check…
        </button>
      );
  }
  const pill = headPill(pr);
  const head = splitChecks(pr).head;
  const sections: readonly { readonly key: Section; readonly word: string }[] = [
    { key: "overview", word: "Overview" },
    { key: "files", word: pr.filesChanged > 0 ? `Files · ${pr.filesChanged}` : "Files" },
    { key: "checks", word: `Checks · ${head.length}` },
  ];
  const settings = props.settings ?? null;
  return (
    <div className="pr-detail">
      <header className="pr-detail-head">
        <div className="pr-detail-title">
          <button type="button" className="pr-crumb" onClick={props.onBack}>
            {props.projectName} › Pull requests ›
          </button>
          <h3 tabIndex={-1} ref={heading}>
            #{pr.number} · {pr.itemId ? <CardLink id={pr.itemId} /> : null}{" "}
            {pr.itemTitle || pr.title}
          </h3>
        </div>
        {pill === null ? null : <Chip text={pill.text} tone={pill.tone} />}
      </header>
      <Byline pr={pr} now={now} />
      <MergeBar pr={pr} now={now} live={props.live ?? true} canUndo={canWrite} owner={owner} />
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
        {section === "overview" ? (
          <PrOverview
            pr={pr}
            now={now}
            ownerAction={ownerAction}
            setting={
              settings === null
                ? null
                : { line: settingLine(settings), onChange: props.onOpenSettings ?? null }
            }
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
            onReview={reviewable ? openReview : null}
            comments={{ ...placed, canWrite, busy: owner.busy, onWrite }}
          />
        ) : null}
        {section === "checks" ? <PrChecks pr={pr} /> : null}
      </div>
      {reviewing === null ? null : (
        <ReviewDialog pr={pr} sha={reviewing} owner={owner} onClose={closeReview} />
      )}
    </div>
  );
}
