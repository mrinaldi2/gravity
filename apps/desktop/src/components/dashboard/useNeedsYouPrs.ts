// The project's pull requests, read only while Needs you lists a review for
// one, so its row can say who approved and whether it is a Re-check.

import { useMemo } from "react";
import type { PullRequest } from "../../protocol/gen/hermes/pr/v1/pr_pb";
import type { PrApi } from "../../protocol/prs";
import { usePrList } from "../prs/usePullRequests";

export function useNeedsYouPrs(
  client: PrApi,
  projectId: string,
  connected: boolean,
  /** Absent when the service serves no pull requests: nothing is read. */
  onOpenPr: unknown,
  dashboard: { readonly needs_you: readonly { readonly kind: string }[] } | null,
): ReadonlyMap<number, PullRequest> | undefined {
  const rows = dashboard === null ? [] : dashboard.needs_you;
  const asked = onOpenPr !== undefined && rows.some((r) => r.kind === "pr_review");
  const list = usePrList(client, projectId, connected && asked);
  return useMemo(
    () => (list.data === null ? undefined : new Map(list.data.map((pr) => [pr.number, pr]))),
    [list.data],
  );
}
