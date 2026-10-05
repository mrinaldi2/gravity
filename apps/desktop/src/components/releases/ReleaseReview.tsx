// The release review (H-018 §4A.2–4A.3): one component for the Releases tab
// and the Decisions view, so there is only one way to rule on a package.

import { useEffect, useRef, useState } from "react";
import type { ReactElement } from "react";
import type { ItemVerdict, Release } from "../../protocol/releases";
import { releaseTitle, statusLabel } from "./labels";
import {
  ApproveDialog,
  HoldDialog,
  LeaveOutDialog,
  PauseDialog,
  RejectDialog,
} from "./ReleaseDialogs";
import type { ReturnTo } from "./ReleaseDialogs";
import { Changelog, Glyph, HowToTest, ItemsList, Rollout, TestSummary } from "./ReleaseSections";
import type { LeftOut } from "./ReleaseSections";
import type { ReleaseActions } from "./useReleases";

type Tab = "items" | "changelog" | "howto" | "rollout";
type Open = "approve" | "hold" | "reject" | "pause" | { readonly leaveOut: string } | null;

const ROLLING: ReadonlySet<string> = new Set([
  "approved",
  "deploying",
  "paused",
  "partially_deployed",
  "deployed",
  "rolled_back",
]);

export interface ReleaseReviewProps {
  readonly release: Release;
  /** Item titles by id, where the board knows them. */
  readonly titles: ReadonlyMap<string, string>;
  readonly botName: (id: string) => string;
  readonly actions: ReleaseActions;
  /** The connection may control the fleet (pause and resume a rollout). */
  readonly canControl: boolean;
  readonly now?: () => number;
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
  const rolling = ROLLING.has(release.status);
  const [tab, setTab] = useState<Tab>(rolling ? "rollout" : "items");
  const [open, setOpen] = useState<Open>(null);
  const [leftOut, setLeftOut] = useState<ReadonlyMap<string, LeftOut>>(new Map());
  const status = statusLabel(release.status);
  const ruling = release.status === "awaiting_owner" || release.status === "held";
  const canRule = release.can_rule === true;
  const failing = release.tests.filter((t) => t.result !== "pass").map((t) => t.machine);
  const total = release.items.length;
  const shipping = total - leftOut.size;
  const item = (id: string): { id: string; title: string } => ({ id, title: titles.get(id) ?? "" });
  const close = (): void => setOpen(null);

  const approve = (): void => {
    const verdicts: ItemVerdict[] = release.items.map((i) => {
      const out = leftOut.get(i.item_id);
      return out
        ? { item_id: i.item_id, verdict: out.verdict, ...(out.note ? { note: out.note } : {}) }
        : { item_id: i.item_id, verdict: "ship" };
    });
    const label = leftOut.size
      ? `Approving ${shipping} of ${total} items of ${version}`
      : `Approving ${version}`;
    actions.rule(release, verdicts, label);
    setLeftOut(new Map());
    close();
  };
  const reject = (reason: string, returns: ReadonlyMap<string, ReturnTo>): void => {
    const verdicts: ItemVerdict[] = release.items.map((i) => ({
      item_id: i.item_id,
      verdict: returns.get(i.item_id) ?? "rework",
      note: reason,
    }));
    actions.rule(release, verdicts, `Rejecting ${version}`);
    close();
  };

  // ⌘↩ approves while the review has focus (§4A.3).
  const root = useRef<HTMLDivElement>(null);
  const approvable = ruling && canRule && shipping > 0 && open === null;
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent): void => {
      const inside = root.current?.contains(document.activeElement) ?? false;
      if (event.key === "Enter" && (event.metaKey || event.ctrlKey) && inside && approvable) {
        event.preventDefault();
        setOpen("approve");
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [approvable]);

  const tabs: readonly { readonly id: Tab; readonly label: string }[] = [
    { id: "items", label: `Items ${total}` },
    { id: "changelog", label: "Changelog" },
    { id: "howto", label: "How to test" },
    ...(rolling ? [{ id: "rollout" as const, label: "Rollout" }] : []),
  ];

  return (
    <div className="release-review" role="region" aria-label={`Release ${version}`} ref={root}>
      <header className="release-head">
        <h2>{version}</h2>
        <span className={`release-pill release-tone-${status.tone}`}>
          <span aria-hidden="true">{status.glyph}</span> {status.word}
        </span>
      </header>
      <p className="release-meta">
        Packaged by {botName(release.created_by)} · <span className="mono">{release.name}</span>
        {release.supersedes ? " · replaces an earlier package" : ""}
      </p>
      <Banner release={release} actions={actions} canControl={canControl} />
      <TestSummary release={release} botName={botName} />
      <div className="release-tabs" role="tablist" aria-label="Package">
        {tabs.map((t) => (
          <button
            key={t.id}
            type="button"
            role="tab"
            aria-selected={tab === t.id}
            className={tab === t.id ? "release-tab on" : "release-tab"}
            onClick={() => setTab(t.id)}
          >
            {t.label}
          </button>
        ))}
      </div>
      <div className="release-panel" role="tabpanel">
        {tab === "items" ? (
          <ItemsList
            release={release}
            titles={titles}
            leftOut={leftOut}
            editable={release.status === "awaiting_owner" && canRule}
            onLeaveOut={(id) => setOpen({ leaveOut: id })}
            onInclude={(id) => {
              const next = new Map(leftOut);
              next.delete(id);
              setLeftOut(next);
            }}
          />
        ) : null}
        {tab === "changelog" ? <Changelog release={release} /> : null}
        {tab === "howto" ? <HowToTest release={release} titles={titles} /> : null}
        {tab === "rollout" ? (
          <div className="release-rollout">
            <Rollout release={release} />
            {canControl && ["deploying", "partially_deployed"].includes(release.status) ? (
              <button type="button" className="btn btn-small" onClick={() => setOpen("pause")}>
                Pause rollout
              </button>
            ) : null}
          </div>
        ) : null}
      </div>
      {ruling ? (
        <footer className="release-bar">
          {actions.pending ? (
            <span className="release-pending" role="status">
              {actions.pending}…{" "}
              <button type="button" className="cc-link" onClick={actions.undo}>
                Undo
              </button>
            </span>
          ) : (
            <span className="release-bar-note">
              {barNote(release, failing, leftOut.size, canRule)}
            </span>
          )}
          <button
            type="button"
            className="btn btn-small btn-danger"
            disabled={!canRule || actions.pending !== null}
            onClick={() => setOpen("reject")}
          >
            Reject…
          </button>
          {release.status === "held" ? (
            <button
              type="button"
              className="btn btn-small"
              disabled={!canRule || actions.pending !== null}
              onClick={() => actions.unhold(release)}
            >
              Take off hold
            </button>
          ) : (
            <button
              type="button"
              className="btn btn-small"
              disabled={!canRule || actions.pending !== null}
              onClick={() => setOpen("hold")}
            >
              Hold
            </button>
          )}
          <button
            type="button"
            className="btn btn-small btn-primary"
            disabled={!canRule || shipping === 0 || actions.pending !== null}
            onClick={() => setOpen("approve")}
          >
            {leftOut.size ? `Approve ${shipping} of ${total} items` : `Approve ${version}`}
          </button>
        </footer>
      ) : null}
      {open === "approve" ? (
        <ApproveDialog
          title={leftOut.size ? `Approve ${shipping} of ${total} items?` : `Approve ${version}?`}
          warning={
            failing.length
              ? `${failing.join(", ")} didn't pass. Approve anyway, or leave the affected items out first.`
              : undefined
          }
          body={
            leftOut.size
              ? `DevOps repackages the ${shipping} approved items without the rest, and you rule on that new build. The items left out go back as you chose.`
              : `DevOps rolls ${version} out to each computer, one at a time, starting now.`
          }
          confirmLabel={leftOut.size ? `Approve ${shipping} items` : "Approve"}
          onConfirm={approve}
          onCancel={close}
        />
      ) : null}
      {open === "hold" ? (
        <HoldDialog
          title={`Hold ${version}?`}
          now={now}
          onConfirm={(note, remindAt) => {
            actions.hold(release, note, remindAt);
            close();
          }}
          onCancel={close}
        />
      ) : null}
      {open === "reject" ? (
        <RejectDialog
          title={`Reject ${version}?`}
          items={release.items.map((i) => item(i.item_id))}
          onConfirm={reject}
          onCancel={close}
        />
      ) : null}
      {open === "pause" ? (
        <PauseDialog
          title={`Pause the rollout of ${version}?`}
          onConfirm={(reason) => {
            actions.pause(release, reason);
            close();
          }}
          onCancel={close}
        />
      ) : null}
      {open !== null && typeof open === "object" ? (
        <LeaveOutDialog
          item={item(open.leaveOut)}
          onConfirm={(verdict, note) => {
            setLeftOut(new Map(leftOut).set(open.leaveOut, { verdict, note }));
            close();
          }}
          onCancel={close}
        />
      ) : null}
    </div>
  );
}

/** What should give the owner pause, or that nothing does (§4A.2). */
function barNote(
  release: Release,
  failing: readonly string[],
  left: number,
  canRule: boolean,
): string {
  if (!canRule) {
    return release.rule_on
      ? `Rule on it from a device connected to ${release.rule_on}.`
      : "This device can't approve: it needs the approve permission.";
  }
  const notes: string[] = [];
  if (failing.length) {
    notes.push(
      `⚠ ${failing.length} computer${failing.length > 1 ? "s" : ""} didn't pass: ${failing.join(", ")}`,
    );
  }
  if (left) {
    notes.push(`${left} item${left > 1 ? "s" : ""} left out`);
  }
  if (notes.length) {
    return notes.join(" · ");
  }
  return release.tests.length
    ? `All ${release.tests.length} computer${release.tests.length > 1 ? "s" : ""} passed.`
    : "No test results yet.";
}

/** The state that changes what the owner can do here. */
function Banner({
  release,
  actions,
  canControl,
}: {
  readonly release: Release;
  readonly actions: ReleaseActions;
  readonly canControl: boolean;
}): ReactElement | null {
  switch (release.status) {
    case "held":
      return (
        <p className="release-banner" role="status">
          <Glyph label={statusLabel("held")} />
          {release.held_note ? ` · ${release.held_note}` : ""}
          {release.remind_at
            ? ` · reminds you ${new Date(release.remind_at).toLocaleString()}`
            : ""}
        </p>
      );
    case "paused":
      return (
        <p className="release-banner" role="status">
          <Glyph label={statusLabel("paused")} />
          {release.paused_reason ? `: ${release.paused_reason}` : ""}
          {canControl ? (
            <button type="button" className="btn btn-small" onClick={() => actions.resume(release)}>
              Resume rollout
            </button>
          ) : null}
        </p>
      );
    case "repackaging": {
      const left = release.items.filter((i) => i.verdict !== "ship").map((i) => i.item_id);
      return (
        <p className="release-banner" role="status">
          You approved part of this package. DevOps is building a new one without {left.join(", ")};
          you'll rule on that build.
        </p>
      );
    }
    case "partially_deployed":
      return (
        <p className="release-banner release-banner-bad" role="status">
          The rollout failed on {failedOn(release).join(", ") || "a computer"}. Its items are back
          in Verify, and DevOps has a rollback task.
        </p>
      );
    case "superseded":
    case "cancelled":
    case "rejected":
    case "rolled_back":
      return (
        <p className="release-banner" role="status">
          <Glyph label={statusLabel(release.status)} />. Nothing more to do here.
        </p>
      );
    default:
      return null;
  }
}

function failedOn(release: Release): string[] {
  return release.deployments
    .filter((d) => d.action === "deploy" && d.result === "failed")
    .map((d) => d.machine);
}
