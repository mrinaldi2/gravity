// Your review of one open PR: whether you can write, the review dialog's
// commit and focus, and the line comments placed under their lines. Every
// write reads the PR and its comments again.

import { useCallback, useMemo, useRef, useState } from "react";
import type { PullRequest } from "../../protocol/gen/hermes/pr/v1/pr_pb";
import { PrState } from "../../protocol/gen/hermes/pr/v1/pr_pb";
import type { DiffComments } from "./FileDiff";
import type { CommentWrite, Thread } from "./LineComments";
import { lineKey, threadsOf } from "./LineComments";
import { usePrComments } from "./usePullRequests";
import type { OwnerAct, PrClient } from "./usePrOwner";
import { useOwnerAct } from "./usePrOwner";

type Placed = Pick<DiffComments, "at" | "outdated">;

/** Threads by the line they sit under on the head, and those whose line is gone. */
function placeThreads(threads: readonly Thread[]): Placed {
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

export interface PrReview {
  readonly owner: OwnerAct;
  /** Connected, with approve, on an open or merging PR. */
  readonly canWrite: boolean;
  /** You can give a verdict: the PR is open. */
  readonly reviewable: boolean;
  readonly comments: Omit<DiffComments, "oldSide">;
  /** The commit the open dialog reviews; null when it's closed. */
  readonly reviewing: string | null;
  readonly openReview: (event: { readonly currentTarget: HTMLElement }) => void;
  readonly closeReview: () => void;
}

function canWriteOn(client: PrClient, pr: PullRequest, connected: boolean): boolean {
  const live = pr.state === PrState.OPEN || pr.state === PrState.MERGING;
  return connected && live && client.hasGrant("approve");
}

export function usePrReview(
  client: PrClient,
  pr: PullRequest,
  connected: boolean,
  reload: () => void,
): PrReview {
  const [reviewing, setReviewing] = useState<string | null>(null);
  const opener = useRef<HTMLElement | null>(null);
  const read = usePrComments(client, pr, connected);
  const reloadComments = read.reload;
  const after = useCallback(() => {
    reload();
    reloadComments();
  }, [reload, reloadComments]);
  const owner = useOwnerAct(client, after);
  const placed = useMemo(() => placeThreads(threadsOf(read.data?.comments ?? [])), [read.data]);
  const canWrite = canWriteOn(client, pr, connected);

  const onWrite = (write: CommentWrite): Promise<boolean> => {
    const target = { project_id: pr.projectId, number: pr.number };
    return owner.act(
      write.type === "pr_comment_resolve"
        ? { ...write, ...target }
        : { ...write, ...target, sha: pr.headSha },
    );
  };
  const closeReview = useCallback((): void => {
    setReviewing(null);
    // Back to the button that opened it, or the PR's heading once that is gone.
    const back =
      opener.current?.isConnected === true
        ? opener.current
        : document.querySelector<HTMLElement>(".pr-detail-title h3");
    back?.focus();
  }, []);
  return {
    owner,
    canWrite,
    reviewable: canWrite && pr.state === PrState.OPEN,
    comments: { ...placed, canWrite, busy: owner.busy, onWrite },
    reviewing,
    openReview: (event) => {
      opener.current = event.currentTarget;
      owner.clear();
      setReviewing(pr.headSha);
    },
    closeReview,
  };
}
