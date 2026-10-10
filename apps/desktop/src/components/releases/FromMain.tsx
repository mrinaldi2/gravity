// A release cut from main (H-278; UX-051 canvas "release", decision 10):
// what's in it, PR by PR with its reviews, what merged unplanned, what
// planned hasn't merged, and the owner's Leave out of one PR.

import { useEffect, useRef, useState } from "react";
import type { ReactElement } from "react";
import type { PrRef, Release } from "../../protocol/releases";
import CardLink from "../cards/CardLink";
import { plural } from "./labels";
import { Dialog } from "./ReleaseDialogs";
import type { PrRecords } from "./releaseMain";
import {
  allPrs,
  canLeaveOut,
  isFromMain,
  leaveOutBody,
  leaveOutPlan,
  reviewChips,
  summaryLine,
} from "./releaseMain";

export interface FromMainProps {
  readonly release: Release;
  /** The release its range starts after, by name; none for the first. */
  readonly previous?: string | null;
  readonly titles: ReadonlyMap<string, string>;
  /** The PRs' own records, for their review chips; none, no chips. */
  readonly records?: PrRecords;
  /** Opens a PR in the Pull requests tab; none, the number is plain text. */
  readonly onOpenPr?: (num: number) => void;
  /** Sends the owner's Leave out of one of its PRs, once confirmed. */
  readonly onLeaveOut: (release: Release, pr: PrRef) => void;
}

const NONE: PrRecords = new Map();

/** "#42 · H-247 Waiting for you on releases": the PR, always with its card. */
function prName(pr: PrRef, titles: ReadonlyMap<string, string>): string {
  return `#${pr.number} · ${pr.item_id} ${pr.title ?? titles.get(pr.item_id) ?? ""}`.trimEnd();
}

/** Included, left out (undone on main), or the undo PR itself. */
function PrState({ pr }: { readonly pr: PrRef }): ReactElement {
  if (pr.reverted) {
    return (
      <span className="release-tone release-tone-off">
        <span aria-hidden="true">⤼</span> Left out · undone on main
      </span>
    );
  }
  if (pr.revert) {
    return (
      <span className="release-tone release-tone-off">
        <span aria-hidden="true">↶</span> Undo pull request
      </span>
    );
  }
  return (
    <span className="release-tone release-tone-ok">
      <span aria-hidden="true">✓</span> Included
    </span>
  );
}

function PrRow(props: {
  readonly pr: PrRef;
  readonly release: Release;
  readonly titles: ReadonlyMap<string, string>;
  readonly records: PrRecords;
  readonly onOpenPr?: (num: number) => void;
  readonly onLeaveOut: (pr: PrRef) => void;
}): ReactElement {
  const { pr, titles, onOpenPr } = props;
  const record = props.records.get(pr.number);
  const title = pr.title ?? titles.get(pr.item_id) ?? "";
  return (
    <li className="release-pr" data-pr={pr.number}>
      {onOpenPr ? (
        <button
          type="button"
          className="release-pr-no cc-link"
          aria-label={`${prName(pr, titles)}, open pull request`}
          onClick={() => onOpenPr(pr.number)}
        >
          #{pr.number}
        </button>
      ) : (
        <span className="release-pr-no">#{pr.number}</span>
      )}
      <div className="release-pr-main">
        <div className="release-pr-title">
          <span className="mono">
            <CardLink id={pr.item_id} />
          </span>{" "}
          {title}
        </div>
        <div className="release-meta">
          merged · <span className="mono">{pr.merged_sha.slice(0, 7)}</span>
        </div>
        {record ? (
          <ul className="release-chips" aria-label={`Reviews of #${pr.number}`}>
            {reviewChips(record).map((c) => (
              <li key={c.text} className={`release-chip release-tone-${c.tone}`}>
                {c.text}
              </li>
            ))}
          </ul>
        ) : null}
      </div>
      <PrState pr={pr} />
      {canLeaveOut(props.release, pr) ? (
        <button
          type="button"
          className="btn btn-small"
          data-leave-out={pr.number}
          aria-label={`Leave out #${pr.number}…`}
          onClick={() => props.onLeaveOut(pr)}
        >
          Leave out…
        </button>
      ) : null}
    </li>
  );
}

/** The confirmation names what happens: a re-cut, or an undo PR through the queue. */
export function PrLeaveOutDialog(props: {
  readonly release: Release;
  readonly pr: PrRef;
  readonly titles: ReadonlyMap<string, string>;
  readonly onConfirm: () => void;
  readonly onCancel: () => void;
}): ReactElement {
  const { release, pr } = props;
  const plan = leaveOutPlan(release, [pr.number]);
  return (
    <Dialog
      title={`Leave out ${prName(pr, props.titles)}?`}
      onCancel={props.onCancel}
      actions={
        <button type="button" className="btn btn-small" onClick={props.onConfirm}>
          Leave out #{pr.number}
        </button>
      }
    >
      <p className="confirm-body">{leaveOutBody(release, pr, plan)}</p>
    </Dialog>
  );
}

/** Nothing for a package made the old way; its items list says what's in it. */
export default function FromMain(props: FromMainProps): ReactElement | null {
  return isFromMain(props.release) ? <WhatsIn {...props} /> : null;
}

function WhatsIn(props: FromMainProps): ReactElement {
  const { release, titles } = props;
  const records = props.records ?? NONE;
  const [asked, setAsked] = useState<PrRef | null>(null);
  const root = useRef<HTMLElement>(null);
  const back = useRef<number | null>(null);
  // Closing the confirmation returns focus to the row's Leave out…, or the
  // row's PR link, else the section's heading (H-276 focus rules).
  useEffect(() => {
    if (asked !== null || back.current === null) {
      return;
    }
    const n = back.current;
    back.current = null;
    const el =
      root.current?.querySelector<HTMLElement>(`[data-leave-out="${n}"]`) ??
      root.current?.querySelector<HTMLElement>(`[data-pr="${n}"] button.release-pr-no`) ??
      root.current?.querySelector<HTMLElement>("h3");
    el?.focus();
  }, [asked]);
  const close = (): void => {
    back.current = asked?.number ?? null;
    setAsked(null);
  };
  const row = (pr: PrRef): ReactElement => (
    <PrRow
      key={pr.number}
      pr={pr}
      release={release}
      titles={titles}
      records={records}
      onOpenPr={props.onOpenPr}
      onLeaveOut={setAsked}
    />
  );
  const also = release.also_included ?? [];
  const notMerged = release.not_merged ?? [];
  return (
    <section className="release-main" aria-label="What's in it" ref={root}>
      <h3 tabIndex={-1}>
        What&apos;s in it · {summaryLine(release, props.previous ?? null, records)}
      </h3>
      {release.prs?.length ? (
        <ul className="release-prs">{release.prs.map(row)}</ul>
      ) : allPrs(release).length === 0 ? (
        <p className="release-hint">No pull request merged since the last release.</p>
      ) : null}
      {also.length ? (
        <>
          <h4>
            Also included · {plural(also.length, "merged pull request")} not planned for this
            release
          </h4>
          <ul className="release-prs">{also.map(row)}</ul>
        </>
      ) : null}
      {notMerged.length ? (
        <>
          <h4>Not merged yet · {plural(notMerged.length, "planned card")}</h4>
          <ul className="release-prs">
            {notMerged.map((id) => (
              <li key={id} className="release-pr">
                <span className="release-pr-no" aria-hidden="true">
                  ○
                </span>
                <div className="release-pr-main">
                  <div className="release-pr-title">
                    <span className="mono">
                      <CardLink id={id} />
                    </span>{" "}
                    {titles.get(id) ?? ""}
                  </div>
                  <div className="release-meta">Planned; its pull request isn&apos;t merged</div>
                </div>
              </li>
            ))}
          </ul>
        </>
      ) : null}
      {asked ? (
        <PrLeaveOutDialog
          release={release}
          pr={asked}
          titles={titles}
          onCancel={close}
          onConfirm={() => {
            props.onLeaveOut(release, asked);
            close();
          }}
        />
      ) : null}
    </section>
  );
}
