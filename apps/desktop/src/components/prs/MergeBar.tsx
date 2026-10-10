// The 10 s Undo (H-261 §16, ruling 7629a873; H-271): once your Approve makes
// a PR mergeable it waits 10 s before the merge is handed to DevOps, and
// nothing is pushed meanwhile, so Undo withdraws your approval with main
// untouched. After the window it says it waits for DevOps (H-284 S3).

import { useEffect, useState } from "react";
import type { ReactElement } from "react";
import type { PullRequest } from "../../protocol/gen/hermes/pr/v1/pr_pb";
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

export default function MergeBar(props: {
  readonly pr: PullRequest;
  readonly now: number;
  /** Ticks every second; off in stories and tests that pin the time. */
  readonly live: boolean;
  readonly canUndo: boolean;
  readonly owner: OwnerAct;
}): ReactElement | null {
  const { pr, owner } = props;
  const now = useSeconds(props.live && mergeWindow(pr, props.now) !== null, props.now);
  const phase = mergeWindow(pr, now);
  if (phase === null) {
    return null;
  }
  if (phase.kind === "devops") {
    return (
      <div className="pr-merge-bar" role="status">
        ◌ Waiting for DevOps to merge #{pr.number} into {pr.base || "main"}.
      </div>
    );
  }
  const undo = props.canUndo && youApproved(pr);
  return (
    <div className="pr-merge-bar pr-merge-bar-undo" role="status">
      <span>
        ◌ Merging #{pr.number} into {pr.base || "main"} in {phase.seconds} s. Nothing is pushed
        until then.
      </span>
      {undo ? (
        <button
          type="button"
          className="btn btn-small"
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
