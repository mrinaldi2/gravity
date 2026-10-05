import { useCallback, useEffect, useState } from "react";
import type { AddToast } from "../../app/useToasts";
import type { DaemonApi } from "../../protocol/api";
import type { Decision } from "../../protocol/decisions";
import type { Release } from "../../protocol/releases";
import { useReleaseActions } from "./useReleases";
import type { ReleaseActions } from "./useReleases";

export interface DecisionRelease {
  /** The package this decision is about, once found; none for other decisions. */
  readonly release: Release | undefined;
  readonly actions: ReleaseActions;
}

/**
 * A release raises one decision (H-017 §1.4), and the release review is the
 * only way to rule on it, so Decisions shows that review for it. A decision
 * doesn't name its release: the project's packages are looked up for one
 * whose `decision_id` is this decision.
 */
export function useDecisionRelease(
  client: DaemonApi,
  connected: boolean,
  decision: Decision | undefined,
  addToast: AddToast,
): DecisionRelease {
  const [release, setRelease] = useState<Release | undefined>(undefined);
  const decisionId = decision?.id;
  const projectId = decision?.project_id;
  useEffect(() => {
    if (!connected || decisionId === undefined || projectId === undefined) {
      return;
    }
    let live = true;
    client
      .request({ type: "list_releases", project_id: projectId }, "releases")
      .then((reply) => {
        if (live) {
          setRelease(reply.releases.find((r) => r.decision_id === decisionId));
        }
      })
      // Not a release decision as far as anyone can tell: show the decision.
      .catch(() => (live ? setRelease(undefined) : undefined));
    return () => {
      live = false;
    };
  }, [client, connected, decisionId, projectId]);
  const replace = useCallback((next: Release) => setRelease(next), []);
  const actions = useReleaseActions(client, addToast, replace);
  return {
    release: release !== undefined && release.decision_id === decisionId ? release : undefined,
    actions,
  };
}
