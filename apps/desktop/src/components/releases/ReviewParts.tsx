// The release review's parts: the package tabs, the state banner and the
// sticky action bar (H-018 §4A.2–4A.4).

import { useState } from "react";
import type { ReactElement } from "react";
import type { Release } from "../../protocol/releases";
import { statusLabel } from "./labels";
import { Changelog, Glyph, HowToTest, ItemsList, Rollout } from "./ReleaseSections";
import type { LeftOut } from "./ReleaseSections";
import type { ReleaseActions } from "./useReleases";

type Tab = "items" | "changelog" | "howto" | "rollout";

const ROLLING: ReadonlySet<string> = new Set([
  "approved",
  "deploying",
  "paused",
  "partially_deployed",
  "deployed",
  "rolled_back",
]);

export function ReviewTabs(props: {
  readonly release: Release;
  readonly titles: ReadonlyMap<string, string>;
  readonly leftOut: ReadonlyMap<string, LeftOut>;
  readonly editable: boolean;
  readonly canControl: boolean;
  readonly onLeaveOut: (itemId: string) => void;
  readonly onInclude: (itemId: string) => void;
  readonly onPause: () => void;
}): ReactElement {
  const { release, titles } = props;
  const rolling = ROLLING.has(release.status);
  const [tab, setTab] = useState<Tab>(rolling ? "rollout" : "items");
  const tabs: readonly { readonly id: Tab; readonly label: string }[] = [
    { id: "items", label: `Items ${release.items.length}` },
    { id: "changelog", label: "Changelog" },
    { id: "howto", label: "How to test" },
    ...(rolling ? [{ id: "rollout" as const, label: "Rollout" }] : []),
  ];
  const pausable = props.canControl && ["deploying", "partially_deployed"].includes(release.status);
  return (
    <>
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
            leftOut={props.leftOut}
            editable={props.editable}
            onLeaveOut={props.onLeaveOut}
            onInclude={props.onInclude}
          />
        ) : null}
        {tab === "changelog" ? <Changelog release={release} /> : null}
        {tab === "howto" ? <HowToTest release={release} titles={titles} /> : null}
        {tab === "rollout" ? (
          <div className="release-rollout">
            <Rollout release={release} />
            {pausable ? (
              <button type="button" className="btn btn-small" onClick={props.onPause}>
                Pause rollout
              </button>
            ) : null}
          </div>
        ) : null}
      </div>
    </>
  );
}

export type BarAction = "approve" | "hold" | "reject";

/** Approve, Hold and Reject, and what should give the owner pause. */
export function ReviewBar(props: {
  readonly release: Release;
  readonly version: string;
  readonly actions: ReleaseActions;
  readonly leftOut: number;
  readonly onOpen: (action: BarAction) => void;
}): ReactElement {
  const { release, actions } = props;
  const canRule = release.can_rule === true;
  const blocked = !canRule || actions.pending !== null;
  const total = release.items.length;
  const shipping = total - props.leftOut;
  return (
    <footer className="release-bar">
      {actions.pending ? (
        <span className="release-pending" role="status">
          {actions.pending}…{" "}
          <button type="button" className="cc-link" onClick={actions.undo}>
            Undo
          </button>
        </span>
      ) : (
        <span className="release-bar-note">{barNote(release, props.leftOut)}</span>
      )}
      <button
        type="button"
        className="btn btn-small btn-danger"
        disabled={blocked}
        onClick={() => props.onOpen("reject")}
      >
        Reject…
      </button>
      {release.status === "held" ? (
        <button
          type="button"
          className="btn btn-small"
          disabled={blocked}
          onClick={() => actions.unhold(release)}
        >
          Take off hold
        </button>
      ) : (
        <button
          type="button"
          className="btn btn-small"
          disabled={blocked}
          onClick={() => props.onOpen("hold")}
        >
          Hold
        </button>
      )}
      <button
        type="button"
        className="btn btn-small btn-primary"
        disabled={blocked || shipping === 0}
        onClick={() => props.onOpen("approve")}
      >
        {props.leftOut ? `Approve ${shipping} of ${total} items` : `Approve ${props.version}`}
      </button>
    </footer>
  );
}

/** The machines whose result isn't a pass. */
export function failingMachines(release: Release): string[] {
  return release.tests.filter((t) => t.result !== "pass").map((t) => t.machine);
}

function plural(n: number, word: string): string {
  return `${n} ${word}${n === 1 ? "" : "s"}`;
}

/** What should give the owner pause, or that nothing does (§4A.2). */
function barNote(release: Release, left: number): string {
  if (release.can_rule !== true) {
    return release.rule_on
      ? `Rule on it from a device connected to ${release.rule_on}.`
      : "This device can't approve: it needs the approve permission.";
  }
  const failing = failingMachines(release);
  const notes: string[] = [];
  if (failing.length) {
    notes.push(`⚠ ${plural(failing.length, "computer")} didn't pass: ${failing.join(", ")}`);
  }
  if (left) {
    notes.push(`${plural(left, "item")} left out`);
  }
  if (notes.length) {
    return notes.join(" · ");
  }
  return release.tests.length
    ? `All ${plural(release.tests.length, "computer")} passed.`
    : "No test results yet.";
}

/** The state that changes what the owner can do here. */
export function Banner(props: {
  readonly release: Release;
  readonly actions: ReleaseActions;
  readonly canControl: boolean;
}): ReactElement | null {
  const { release } = props;
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
          {props.canControl ? (
            <button
              type="button"
              className="btn btn-small"
              onClick={() => props.actions.resume(release)}
            >
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
