import { useCallback, useEffect, useState } from "react";
import type { DaemonApi } from "../../protocol/api";
import type { InstallOffer, Release, ReleaseInstall } from "../../protocol/releases";
import { errText } from "../../util";
import { installStage } from "./install";

export interface ReleaseInstallState {
  /** What the daemon says the package offers; null until it answers. */
  readonly info: ReleaseInstall | null;
  /** Why it couldn't be read, if it couldn't. */
  readonly error: string | null;
  readonly send: (deviceId: string) => Promise<InstallOffer>;
}

/**
 * The Install box's facts for a package with a published iOS build (H-229),
 * read with a check of the build site once the package offers install.
 */
export function useReleaseInstall(
  client: DaemonApi | undefined,
  release: Release,
): ReleaseInstallState {
  const [info, setInfo] = useState<ReleaseInstall | null>(null);
  const [error, setError] = useState<string | null>(null);
  const wanted = client !== undefined && installStage(release) === "offered";
  // The review is keyed by package, so a new package starts afresh.
  const { id } = release;
  useEffect(() => {
    if (!wanted) {
      return;
    }
    let live = true;
    const load = async (): Promise<void> => {
      try {
        const reply = await client.request(
          { type: "release_install", release_id: id, check_site: true },
          "release_install",
        );
        if (live) {
          setInfo(reply.install);
          setError(null);
        }
      } catch (e) {
        if (live) {
          setError(errText(e));
        }
      }
    };
    void load();
    return () => {
      live = false;
    };
  }, [client, wanted, id]);
  const send = useCallback(
    async (deviceId: string): Promise<InstallOffer> => {
      if (client === undefined) {
        throw new Error("not connected");
      }
      const reply = await client.request(
        { type: "release_send_to_device", release_id: id, device_id: deviceId },
        "install_offer",
      );
      return reply.offer;
    },
    [client, id],
  );
  return { info, error, send };
}
