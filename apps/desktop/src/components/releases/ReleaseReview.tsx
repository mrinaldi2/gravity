// The release review (H-018 §4A.2–4A.3): one component for the Releases tab
// and the Decisions view, so there is only one way to rule on a package.

import { useEffect, useRef, useState } from "react";
import type { ReactElement } from "react";
import type { ItemVerdict, Release } from "../../protocol/releases";
import { plural, releaseTitle, statusLabel } from "./labels";
import type { BotName } from "./labels";
import {
  ApproveDialog,
  HoldDialog,
  LeaveOutDialog,
  PauseDialog,
  RejectDialog,
} from "./ReleaseDialogs";
import type { ReturnTo } from "./ReleaseDialogs";
import { TestSummary } from "./ReleaseSections";
import type { LeftOut } from "./ReleaseSections";
import { Banner, ReviewBar, ReviewEvents, ReviewTabs, failingMachines } from "./ReviewParts";
import type { BarAction } from "./ReviewParts";
import type { ReleaseActions } from "./useReleases";

type Open = BarAction | "pause" | { readonly leaveOut: string } | null;

export interface ReleaseReviewProps {
  readonly release: Release;
  /** Item titles by id, where the board knows them. */
  readonly titles: ReadonlyMap<string, string>;
  readonly botName: BotName;
  readonly actions: ReleaseActions;
  /** The connection may control the fleet (pause and resume a rollout). */
  readonly canControl: boolean;
  readonly now?: () => number;
}

/** The verdicts for an approval: every item ships unless left out. */
function approval(release: Release, leftOut: ReadonlyMap<string, LeftOut>): ItemVerdict[] {
  return release.items.map((i) => {
    const out = leftOut.get(i.item_id);
    return out
      ? { item_id: i.item_id, verdict: out.verdict, ...(out.note ? { note: out.note } : {}) }
      : { item_id: i.item_id, verdict: "ship" };
  });
}

/** The verdicts for a rejection: the reason on every item, each where the owner sent it. */
function rejection(
  release: Release,
  reason: string,
  returns: ReadonlyMap<string, ReturnTo>,
): ItemVerdict[] {
  return release.items.map((i) => ({
    item_id: i.item_id,
    verdict: returns.get(i.item_id) ?? "rework",
    note: reason,
  }));
}

/** ⌘↩ opens the approval while the review has focus (§4A.3). */
function useApproveKey(enabled: boolean, onApprove: () => void) {
  const root = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent): void => {
      const inside = root.current?.contains(document.activeElement) ?? false;
      if (event.key === "Enter" && (event.metaKey || event.ctrlKey) && inside && enabled) {
        event.preventDefault();
        onApprove();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [enabled, onApprove]);
  return root;
}

export default function ReleaseReview({
  release,
  titles,
  botName,
  actions,
  canControl,
  now = Date.now,
}: ReleaseReviewProps): ReactElement {
  const version = releaseTitle(release);
  const [open, setOpen] = useState<Open>(null);
  const [leftOut, setLeftOut] = useState<ReadonlyMap<string, LeftOut>>(new Map());
  const status = statusLabel(release.status);
  const ruling = release.status === "awaiting_owner" || release.status === "held";
  const canRule = release.can_rule === true;
  const shipping = release.items.length - leftOut.size;
  const root = useApproveKey(ruling && canRule && shipping > 0 && open === null, () =>
    setOpen("approve"),
  );

  return (
    <div className="release-review" role="region" aria-label={`Release ${version}`} ref={root}>
      <header className="release-head">
        <h2>{version}</h2>
        <span className={`release-pill release-tone-${status.tone}`}>
          <span aria-hidden="true">{status.glyph}</span> {status.word}
        </span>
      </header>
      <p className="release-meta">
        Packaged by {botName(release.created_by) ?? "a bot"} ·{" "}
        <span className="mono">{release.name}</span>
        {release.supersedes ? " · replaces an earlier package" : ""}
      </p>
      <Banner release={release} actions={actions} canControl={canControl} />
      <ReviewEvents release={release} botName={botName} />
      <TestSummary release={release} botName={botName} />
      <ReviewTabs
        release={release}
        titles={titles}
        leftOut={leftOut}
        editable={release.status === "awaiting_owner" && canRule}
        canControl={canControl}
        onLeaveOut={(id) => setOpen({ leaveOut: id })}
        onInclude={(id) => {
          const next = new Map(leftOut);
          next.delete(id);
          setLeftOut(next);
        }}
        onPause={() => setOpen("pause")}
      />
      {ruling ? (
        <ReviewBar
          release={release}
          version={version}
          actions={actions}
          leftOut={leftOut.size}
          onOpen={setOpen}
        />
      ) : null}
      <ReviewDialogs
        open={open}
        release={release}
        version={version}
        titles={titles}
        leftOut={leftOut}
        now={now}
        onClose={() => setOpen(null)}
        onApprove={() => {
          const label = leftOut.size
            ? `Approving ${shipping} of ${release.items.length} items of ${version}`
            : `Approving ${version}`;
          actions.rule(release, approval(release, leftOut), label);
          setLeftOut(new Map());
        }}
        onReject={(reason, returns) =>
          actions.rule(release, rejection(release, reason, returns), `Rejecting ${version}`)
        }
        onHold={(note, remindAt) => actions.hold(release, note, remindAt)}
        onPause={(reason) => actions.pause(release, reason)}
        onLeaveOut={(id, out) => setLeftOut(new Map(leftOut).set(id, out))}
      />
    </div>
  );
}

function ReviewDialogs(props: {
  readonly open: Open;
  readonly release: Release;
  readonly version: string;
  readonly titles: ReadonlyMap<string, string>;
  readonly leftOut: ReadonlyMap<string, LeftOut>;
  readonly now: () => number;
  readonly onClose: () => void;
  readonly onApprove: () => void;
  readonly onReject: (reason: string, returns: ReadonlyMap<string, ReturnTo>) => void;
  readonly onHold: (note: string, remindAt: string | null) => void;
  readonly onPause: (reason: string) => void;
  readonly onLeaveOut: (itemId: string, out: LeftOut) => void;
}): ReactElement | null {
  const { open, release, version, onClose } = props;
  const item = (id: string) => ({ id, title: props.titles.get(id) ?? "" });
  const then =
    <A extends unknown[]>(run: (...args: A) => void) =>
    (...args: A): void => {
      run(...args);
      onClose();
    };
  if (open === null) {
    return null;
  }
  if (typeof open === "object") {
    return (
      <LeaveOutDialog
        item={item(open.leaveOut)}
        onConfirm={then((verdict: ReturnTo, note: string) =>
          props.onLeaveOut(open.leaveOut, { verdict, note }),
        )}
        onCancel={onClose}
      />
    );
  }
  if (open === "hold") {
    return (
      <HoldDialog
        title={`Hold ${version}?`}
        now={props.now}
        onConfirm={then(props.onHold)}
        onCancel={onClose}
      />
    );
  }
  if (open === "reject") {
    return (
      <RejectDialog
        title={`Reject ${version}?`}
        items={release.items.map((i) => item(i.item_id))}
        onConfirm={then(props.onReject)}
        onCancel={onClose}
      />
    );
  }
  if (open === "pause") {
    return (
      <PauseDialog
        title={`Pause the rollout of ${version}?`}
        onConfirm={then(props.onPause)}
        onCancel={onClose}
      />
    );
  }
  const total = release.items.length;
  const left = props.leftOut.size;
  const failing = failingMachines(release);
  return (
    <ApproveDialog
      title={left ? `Approve ${total - left} of ${total} items?` : `Approve ${version}?`}
      warning={
        failing.length
          ? `${failing.join(", ")} didn't pass. Approve anyway, or leave the affected items out first.`
          : undefined
      }
      body={
        left
          ? `DevOps repackages the ${plural(total - left, "approved item")} without the rest, and you rule on that new build. The items left out go back as you chose.`
          : `DevOps rolls ${version} out to each computer, one at a time, starting now.`
      }
      confirmLabel={left ? `Approve ${plural(total - left, "item")}` : `Approve ${version}`}
      onConfirm={then(props.onApprove)}
      onCancel={onClose}
    />
  );
}
