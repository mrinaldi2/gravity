// The PRs' own records for a release cut from main (H-278): `pr_get` for
// each PR it lists, read again on that PR's `pr_updated`, so the review
// chips follow the PR tab. A daemon without pull requests sends none, and
// the screen shows no chips.

import { useEffect, useState } from "react";
import type { DaemonApi } from "../../protocol/api";
import { PULL_REQUESTS, prCall, pushProject } from "../../protocol/prs";
import type { Release } from "../../protocol/releases";
import type { PrRecords } from "./releaseMain";
import { allPrs } from "./releaseMain";

const NONE: PrRecords = new Map();

export function usePrRecords(
  client: DaemonApi,
  projectId: string,
  release: Release | undefined,
  connected: boolean,
): PrRecords {
  const numbers = release ? allPrs(release).map((p) => p.number) : [];
  const wanted = client.capabilities.includes(PULL_REQUESTS) && connected;
  const key = wanted ? `${projectId}:${numbers.join(",")}` : "";
  // Records read for another release or project are dropped by their key.
  const [state, setState] = useState<{ readonly key: string; readonly records: PrRecords }>({
    key: "",
    records: NONE,
  });
  useEffect(() => {
    const shown = new Set(numbers);
    if (key === "" || shown.size === 0) {
      return undefined;
    }
    let live = true;
    const read = async (number: number): Promise<void> => {
      try {
        const pr = await prCall(client, { case: "prGet", value: { projectId, number } }, "pr");
        if (live) {
          setState((was) => ({
            key,
            records: new Map(was.key === key ? was.records : NONE).set(number, pr),
          }));
        }
      } catch {
        // A PR the service can't read just shows no chips.
      }
    };
    const off = client.onPrPush((push) => {
      if (
        push.push.case === "prUpdated" &&
        pushProject(push) === projectId &&
        shown.has(push.push.value.number)
      ) {
        void read(push.push.value.number);
      }
    });
    shown.forEach((n) => void read(n));
    return () => {
      live = false;
      off();
    };
    // `key` names the numbers, so a new array with the same PRs reads nothing again.
    // oxlint-disable-next-line react-hooks/exhaustive-deps
  }, [client, projectId, key]);
  return state.key === key ? state.records : NONE;
}
