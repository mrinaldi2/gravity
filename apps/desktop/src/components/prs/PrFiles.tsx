// A PR's files and unified diff (UX-051 "files"). "Since <sha>" shows only
// what changed after an approval, so a re-check reads the delta (UX-051
// decisions 3 and 11; `pr_diff` with `from_sha`). Line comments sit under
// their lines; the owner can start a thread, reply and resolve (H-282).

import { useState } from "react";
import type { ReactElement } from "react";
import type { DiffFile, PullRequest } from "../../protocol/gen/hermes/pr/v1/pr_pb";
import { FileStatus, Verdict } from "../../protocol/gen/hermes/pr/v1/pr_pb";
import { parseDiff } from "./diffParse";
import type { DiffComments } from "./FileDiff";
import FileDiff from "./FileDiff";
import { roleLabel, sha7 } from "./prText";
import { usePrDiff } from "./usePullRequests";
import type { PrClient } from "./usePrOwner";

/** The commits a re-check can start from: each older approval's, with who gave it. */
function deltaBases(pr: PullRequest): { readonly sha: string; readonly by: string }[] {
  const bases = new Map<string, string[]>();
  for (const review of pr.reviews) {
    if (review.verdict === Verdict.APPROVED && review.sha !== pr.headSha) {
      bases.set(review.sha, [...(bases.get(review.sha) ?? []), roleLabel(review.role)]);
    }
  }
  return [...bases].map(([sha, by]) => ({ sha, by: by.join(", ") }));
}

const STATUS_WORDS: Readonly<Record<FileStatus, string>> = {
  [FileStatus.UNSPECIFIED]: "",
  [FileStatus.ADDED]: "added",
  [FileStatus.MODIFIED]: "",
  [FileStatus.DELETED]: "deleted",
  [FileStatus.RENAMED]: "renamed",
};

function FileButton(props: {
  readonly file: DiffFile;
  readonly on: boolean;
  readonly onPick: () => void;
}): ReactElement {
  const { file } = props;
  const status = STATUS_WORDS[file.status];
  return (
    <li>
      <button
        type="button"
        className={`pr-file ${props.on ? "pr-file-on" : ""}`}
        aria-pressed={props.on}
        title={file.oldPath ? `${file.oldPath} → ${file.path}` : file.path}
        onClick={props.onPick}
      >
        <span className="pr-file-path">{file.path}</span>
        <span className="pr-file-counts">
          {status ? <span className="pr-dim">{status} </span> : null}
          {file.binary ? (
            <span className="pr-dim">binary</span>
          ) : (
            <>
              <span className="pr-tone-ok">+{file.additions}</span>{" "}
              <span className="pr-tone-bad">−{file.deletions}</span>
            </>
          )}
        </span>
      </button>
    </li>
  );
}

/** "Show: Every change, from main" or "Since c79c8b5, approved by CE". */
function DeltaPicker(props: {
  readonly pr: PullRequest;
  readonly from: string | null;
  readonly onFrom: (from: string | null) => void;
}): ReactElement | null {
  const bases = deltaBases(props.pr);
  if (bases.length === 0) {
    return null;
  }
  return (
    <label className="pr-delta">
      Show
      <select
        value={props.from ?? ""}
        onChange={(event) => props.onFrom(event.target.value === "" ? null : event.target.value)}
      >
        <option value="">Every change, from {props.pr.base}</option>
        {bases.map((base) => (
          <option key={base.sha} value={base.sha}>
            Since {sha7(base.sha)}, approved by {base.by}
          </option>
        ))}
      </select>
    </label>
  );
}

/** A re-check shows only what changed since you approved, with Review… to give your verdict. */
function RecheckBanner(props: {
  readonly base: string;
  readonly files: number | null;
  readonly onReview: ((event: { readonly currentTarget: HTMLElement }) => void) | null;
}): ReactElement {
  return (
    <div className="pr-banner pr-banner-recheck" role="status">
      <span>
        <b>⟳ Only what changed since you approved </b>
        <span className="pr-sha">{sha7(props.base)}</span>
        {props.files === null ? "" : ` · ${props.files} files`}
      </span>
      {props.onReview === null ? null : (
        <button type="button" className="btn btn-small btn-primary" onClick={props.onReview}>
          Review…
        </button>
      )}
    </div>
  );
}

export default function PrFiles(props: {
  readonly client: PrClient;
  readonly pr: PullRequest;
  readonly connected: boolean;
  /** The commit the diff starts from; null = from main. */
  readonly from: string | null;
  readonly onFrom: (from: string | null) => void;
  /** Your approval this delta starts from, when it's a re-check. */
  readonly recheck: string | null;
  readonly onReview: ((event: { readonly currentTarget: HTMLElement }) => void) | null;
  readonly comments: Omit<DiffComments, "oldSide">;
}): ReactElement {
  const { pr, from } = props;
  const [picked, setPicked] = useState<string | null>(null);
  const { data, error } = usePrDiff(props.client, pr, from, props.connected);
  const texts = data === null ? [] : parseDiff(data.diff);
  const shown = picked === null ? texts : texts.filter((t) => t.path === picked);
  const range = data === null ? "" : `${sha7(data.fromSha)} → ${sha7(data.toSha)}`;
  const comments: DiffComments = { ...props.comments, oldSide: from === null };
  return (
    <div className="pr-files">
      <div className="pr-file-side">
        <DeltaPicker
          pr={pr}
          from={from}
          onFrom={(next) => {
            props.onFrom(next);
            setPicked(null);
          }}
        />
        {data === null ? (
          <p className="pr-dim">{error ?? "Loading files…"}</p>
        ) : (
          <ul className="pr-plain" aria-label="Files">
            {data.files.map((file) => (
              <FileButton
                key={file.path}
                file={file}
                on={picked === file.path}
                onPick={() => setPicked((was) => (was === file.path ? null : file.path))}
              />
            ))}
          </ul>
        )}
      </div>
      <div className="pr-diff-pane">
        {props.recheck !== null && from === props.recheck ? (
          <RecheckBanner
            base={props.recheck}
            files={data?.files.length ?? null}
            onReview={props.onReview}
          />
        ) : null}
        {data?.truncated ? (
          <p className="pr-banner pr-banner-warn">
            ⚠ The diff is cut at 2 MB. Open the branch in a terminal for the rest.
          </p>
        ) : null}
        {shown.map((file) => (
          <FileDiff key={file.path} file={file} range={range} comments={comments} />
        ))}
      </div>
    </div>
  );
}
