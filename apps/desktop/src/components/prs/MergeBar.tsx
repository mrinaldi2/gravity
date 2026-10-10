// The 10 s Undo (H-261 §16, ruling 7629a873; H-271): once your Approve makes
// a PR mergeable it waits 10 s before the merge is handed to DevOps, and
// nothing is pushed meanwhile, so Undo withdraws your approval with main
// untouched. After the window it says it waits for DevOps (H-284 S3).
// Screen readers hear it once when the window opens and once on the outcome,
// not every second: the counter is aria-hidden (UX-053 should-fix 2).

import { useEffect, useState } from "react";
import type { ReactElement } from "react";
import type { PullRequest } from "../../protocol/gen/hermes/pr/v1/pr_pb";
import { PrState } from "../../protocol/gen/hermes/pr/v1/pr_pb";
import type { MergeWindow } from "./ownerText";
import { mergeWindow, youApproved } from "./ownerText";
import type { OwnerAct } from "./usePrOwner";

/** The time, every second while a window is open. */
function useSeconds(on: boolean, start: number): number {
  const [now, setNow] = useState(start);
  useEffect(() => {
    if (!on) {
      return undefined;
    }
    const timer = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(timer);
  }, [on]);
  return on ? now : start;
}

type Stage = "undo" | "devops" | "merged" | null;

function stageOf(pr: PullRequest, phase: MergeWindow): Stage {
  if (phase !== null) {
    return phase.kind;
  }
  return pr.state === PrState.MERGED ? "merged" : null;
}

/** What the status region says on entering a stage, coming from `from`. */
function announcement(pr: PullRequest, phase: MergeWindow, from: Stage, undo: boolean): string {
  if (phase?.kind === "undo") {
    const seconds = phase.seconds === 1 ? "1 second" : `${phase.seconds} seconds`;
    const then = undo ? " Undo is available." : "";
    return `Merging #${pr.number} into ${pr.base || "main"} in ${seconds}.${then}`;
  }
  if (phase?.kind === "devops") {
    return `Waiting for DevOps to merge #${pr.number}.`;
  }
  if (from === null) {
    return "";
  }
  if (pr.state === PrState.MERGED) {
    return "Merged.";
  }
  return from === "undo" ? "Undone." : "";
}

/** The status text, changed only when the stage does, so the ticking isn't read out. */
function useAnnouncement(pr: PullRequest, phase: MergeWindow, undo: boolean): string {
  const stage = stageOf(pr, phase);
  const [said, setSaid] = useState<{ readonly stage: Stage; readonly text: string }>({
    stage: null,
    text: "",
  });
  if (stage !== said.stage) {
    setSaid({ stage, text: announcement(pr, phase, said.stage, undo) });
  }
  return said.text;
}

function UndoBar(props: {
  readonly pr: PullRequest;
  readonly seconds: number;
  readonly undo: boolean;
  readonly owner: OwnerAct;
}): ReactElement {
  const { pr, owner } = props;
  return (
    <div className="pr-merge-bar pr-merge-bar-undo">
      <span>
        ◌ Merging #{pr.number} into {pr.base || "main"}
        <span aria-hidden="true"> in {props.seconds} s</span>. Nothing is pushed until then.
      </span>
      {props.undo ? (
        <button
          type="button"
          className="btn btn-small"
          aria-label={`Undo your approval of #${pr.number}`}
          disabled={owner.busy}
          onClick={() =>
            void owner.act({ type: "pr_merge_undo", project_id: pr.projectId, number: pr.number })
          }
        >
          Undo
        </button>
      ) : null}
      {owner.error ? <span className="pr-tone-bad">{owner.error}</span> : null}
    </div>
  );
}

export default function MergeBar(props: {
  readonly pr: PullRequest;
  readonly now: number;
  /** Ticks every second; off in stories and tests that pin the time. */
  readonly live: boolean;
  readonly canUndo: boolean;
  readonly owner: OwnerAct;
}): ReactElement {
  const { pr, owner } = props;
  const now = useSeconds(props.live && mergeWindow(pr, props.now) !== null, props.now);
  const phase = mergeWindow(pr, now);
  const undo = props.canUndo && youApproved(pr);
  const said = useAnnouncement(pr, phase, undo);
  return (
    <>
      <span className="visually-hidden" aria-live="polite" aria-atomic="true">
        {said}
      </span>
      {phase?.kind === "undo" ? (
        <UndoBar pr={pr} seconds={phase.seconds} undo={undo} owner={owner} />
      ) : null}
      {phase?.kind === "devops" ? (
        <div className="pr-merge-bar">
          ◌ Waiting for DevOps to merge #{pr.number} into {pr.base || "main"}.
        </div>
      ) : null}
    </>
  );
}
