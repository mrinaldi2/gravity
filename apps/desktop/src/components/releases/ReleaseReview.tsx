// The release review (H-018 §4A.2–4A.3): one component for the Releases tab
// and the Decisions view, so there is only one way to rule on a package.

import { useEffect, useRef, useState } from "react";
import type { ReactElement } from "react";
import type { DaemonApi } from "../../protocol/api";
import type { ItemVerdict, Release } from "../../protocol/releases";
import InstallBox from "./InstallBox";
import { useReleaseInstall } from "./useReleaseInstall";
import { plural, releaseTitle, statusLabel } from "./labels";
import type { BotName } from "./labels";
import {
  ApproveDialog,
  HoldDialog,
  LeaveOutDialog,
  PauseDialog,
  RejectDialog,
} from "./ReleaseDialogs";
import { PostInstall } from "./PostInstall";
import ReleaseProgress from "./ReleaseProgress";
import type { ReturnTo } from "./ReleaseDialogs";
import { TestSummary } from "./ReleaseSections";
import type { LeftOut } from "./ReleaseSections";
import { Banner, ReviewBar, ReviewEvents, ReviewTabs, failingMachines } from "./ReviewParts";
import type { BarAction } from "./ReviewParts";
import type { ReleaseActions } from "./useReleases";
import WaitingForYou, { NowLine } from "./WaitingForYou";
import type { WaitingActions } from "./WaitingForYou";

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
  /** Reads and sends an iOS package's install link (H-229); none, no Install box. */
  readonly client?: DaemonApi;
  /** Where "Waiting for you"'s buttons go (H-247); none, only the ruling row's. */
  readonly waiting?: WaitingActions;
}

/** The release's own Approve / Hold / Reject, scrolled to and focused. */
function focusReview(root: HTMLElement | null): void {
  const target = root?.querySelector<HTMLElement>(".release-bar button:not([disabled])");
  target?.scrollIntoView({ block: "nearest" });
  target?.focus();
}

/** The verdicts for an approval: every item ships unless left out. */
/**
 * What the builds were made from, the commit that lands on main if the
 * owner approves (ARCH-R52): "Built from 1a2b3c4 on release/desktop-0.17.0".
 */
export function sourceLine(release: Release, version: string): string {
  const commits = new Set(release.builds.map((b) => b.source_commit ?? null));
  if (release.builds.length === 0) {
    return "No builds yet";
  }
  if (commits.has(null)) {
    return "Built from: not recorded for every build, so it can't land on main as is";
  }
  if (commits.size > 1) {
    return "Built from more than one commit, so it can't land on main as is";
  }
  const [commit] = [...commits];
  return `Built from ${String(commit).slice(0, 7)} on release/desktop-${version.replace(/^v/u, "")}`;
}

/** Who narrowed a frozen set, if anyone. */
function chosenBy(who: string | null | undefined): string {
  if (who === "lead") {
    return " (chosen by the lead)";
  }
  return who === "owner" ? " (chosen by you)" : "";
}

/**
 * The computers frozen into the package at submit (ARCH-R55): where it was
 * tested, who narrowed that, and where it goes. Null before submit.
 */
export function targetsLine(release: Release): string | null {
  const tested = release.tested_on ?? [];
  const deploys = release.deploys_to ?? [];
  if (tested.length === 0 && deploys.length === 0) {
    return null;
  }
  const by = chosenBy;
  return `Tested on ${tested.join(", ")}${by(release.tested_set_by)} · Goes to ${deploys.join(", ")}${by(release.deploys_set_by)}`;
}

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

/**
 * Who made the package, what it was built from and where it goes. A planned
 * package has no builds yet, so it says nothing about them (H-142).
 */
function PackageFacts({
  release,
  version,
  botName,
}: {
  readonly release: Release;
  readonly version: string;
  readonly botName: BotName;
}): ReactElement {
  const planned = release.status === "planned";
  const targets = targetsLine(release);
  return (
    <>
      <p className="release-meta">
        {planned ? "Planned" : "Packaged"} by {botName(release.created_by) ?? "a bot"} ·{" "}
        <span className="mono">{release.name}</span>
        {release.supersedes ? " · replaces an earlier package" : ""}
      </p>
      {planned ? null : <p className="release-meta">{sourceLine(release, version)}</p>}
      {targets ? <p className="release-meta">{targets}</p> : null}
    </>
  );
}

export default function ReleaseReview({
  release,
  titles: boardTitles,
  botName,
  actions,
  canControl,
  now = Date.now,
  client,
  waiting,
}: ReleaseReviewProps): ReactElement {
  const install = useReleaseInstall(client, release);
  const installBox = client ? <InstallBox release={release} install={install} now={now} /> : null;
  // The package's own live titles (H-137) fill what the board fetch lacks.
  const titles = new Map([
    ...boardTitles,
    ...(release.plan ?? []).map((p): [string, string] => [p.item_id, p.title]),
  ]);
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
      <WaitingForYou
        release={release}
        botName={botName}
        actions={waiting}
        onReview={() => focusReview(root.current)}
        now={now}
      />
      {installBox}
      <PackageFacts release={release} version={version} botName={botName} />
      <Banner release={release} actions={actions} canControl={canControl} />
      <NowLine release={release} botName={botName} onNeedsYou={waiting?.onNeedsYou} />
      <ReleaseProgress release={release} botName={botName} />
      <ReviewEvents release={release} botName={botName} />
      <PostInstall release={release} botName={botName} />
      {release.status === "planned" ? null : <TestSummary release={release} botName={botName} />}
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
        install={installBox}
        phones={install.info?.devices}
        now={now}
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
