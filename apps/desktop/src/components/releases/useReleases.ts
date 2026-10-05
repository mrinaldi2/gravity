import { useCallback, useState } from "react";
import type { AddToast } from "../../app/useToasts";
import { useLoadOnConnect } from "../../hooks/useLoadOnConnect";
import type { DaemonApi } from "../../protocol/api";
import type { ItemVerdict, Release } from "../../protocol/releases";
import { errText } from "../../util";
import { releaseTitle } from "./labels";
import { useDelayedSend } from "./useDelayedSend";

export interface ReleaseActions {
  /** The ruling waiting out its Undo, if any. */
  readonly pending: string | null;
  readonly undo: () => void;
  readonly rule: (release: Release, verdicts: readonly ItemVerdict[], label: string) => void;
  readonly hold: (release: Release, note: string, remindAt: string | null) => void;
  readonly unhold: (release: Release) => void;
  readonly pause: (release: Release, reason: string) => void;
  readonly resume: (release: Release) => void;
}

/**
 * The owner's actions on a package. A ruling, a hold and a pause wait five
 * seconds for Undo before they are sent; taking a hold off or resuming a
 * rollout undoes nothing, so it goes at once.
 */
export function useReleaseActions(
  client: DaemonApi,
  addToast: AddToast,
  onChanged: (release: Release) => void,
): ReleaseActions {
  const delayed = useDelayedSend(addToast);
  const send = useCallback(
    async (body: Parameters<DaemonApi["request"]>[0], failure: string): Promise<void> => {
      try {
        const reply = await client.request(body, "release");
        onChanged(reply.release);
      } catch (error) {
        addToast("error", failure, errText(error));
      }
    },
    [client, addToast, onChanged],
  );
  const { schedule } = delayed;

  return {
    pending: delayed.pending,
    undo: delayed.undo,
    rule: (release, verdicts, label) =>
      schedule(label, () =>
        send(
          {
            type: "release_rule",
            release_id: release.id,
            verdicts,
            expected_version: release.version,
          },
          `Couldn't rule on ${releaseTitle(release)}`,
        ),
      ),
    hold: (release, note, remindAt) =>
      schedule(`Holding ${releaseTitle(release)}`, () =>
        send(
          {
            type: "release_hold",
            release_id: release.id,
            ...(note.trim() ? { note: note.trim() } : {}),
            ...(remindAt ? { remind_at: remindAt } : {}),
          },
          `Couldn't hold ${releaseTitle(release)}`,
        ),
      ),
    unhold: (release) =>
      void send(
        { type: "release_unhold", release_id: release.id },
        `Couldn't take ${releaseTitle(release)} off hold`,
      ),
    pause: (release, reason) =>
      schedule(`Pausing ${releaseTitle(release)}`, () =>
        send(
          { type: "release_pause", release_id: release.id, reason },
          `Couldn't pause ${releaseTitle(release)}`,
        ),
      ),
    resume: (release) =>
      void send(
        { type: "release_resume", release_id: release.id },
        `Couldn't resume ${releaseTitle(release)}`,
      ),
  };
}

export interface ProjectReleases {
  readonly releases: readonly Release[];
  readonly loaded: boolean;
  readonly replace: (release: Release) => void;
}

/** A project's packages, newest first, kept current by the actions. */
export function useProjectReleases(
  client: DaemonApi,
  connected: boolean,
  projectId: string,
  addToast: AddToast,
): ProjectReleases {
  const [releases, setReleases] = useState<readonly Release[]>([]);
  const [loaded, setLoaded] = useState(false);
  const load = useCallback(async (): Promise<void> => {
    try {
      const reply = await client.request(
        { type: "list_releases", project_id: projectId },
        "releases",
      );
      setReleases(reply.releases);
    } catch (error) {
      addToast("error", "Couldn't load the releases", errText(error));
    } finally {
      setLoaded(true);
    }
  }, [client, projectId, addToast]);
  useLoadOnConnect(connected, load);
  const replace = useCallback((release: Release): void => {
    setReleases((all) =>
      all.some((r) => r.id === release.id)
        ? all.map((r) => (r.id === release.id ? release : r))
        : [release, ...all],
    );
  }, []);
  return { releases, loaded, replace };
}
