// Pull requests, read live (H-273): each read subscribes the connection to the
// project's pushes, and a push about what is shown reads it again. Reads that
// overlap a push are coalesced, so a burst of pushes costs one more read.

import { useEffect, useRef, useState } from "react";
import type { MutableRefObject } from "react";
import { useLatestRef } from "../../app/useLatestRef";
import { errText } from "../../util";
import type { PrApi } from "../../protocol/prs";
import { prCall, pushProject } from "../../protocol/prs";
import type { PrDiff, PrPush, PullRequest } from "../../protocol/gen/hermes/pr/v1/pr_pb";
import { PrState } from "../../protocol/gen/hermes/pr/v1/pr_pb";

export interface Loaded<T> {
  readonly data: T | null;
  readonly error: string | null;
}

interface Source<T> {
  readonly load: () => Promise<T>;
  readonly matches: (push: PrPush) => boolean;
}

/**
 * Reads now and on each matching push until the returned function stops it.
 * A read asked for while one is in flight runs once after it.
 */
function follow<T>(
  api: PrApi,
  source: MutableRefObject<Source<T>>,
  onData: (data: T) => void,
  onError: (error: unknown) => void,
): () => void {
  let live = true;
  let busy = false;
  let again = false;
  const read = (): void => {
    if (busy) {
      again = true;
      return;
    }
    busy = true;
    source.current
      .load()
      .then(
        (data) => live && onData(data),
        (error: unknown) => live && onError(error),
      )
      .finally(() => {
        busy = false;
        if (live && again) {
          again = false;
          read();
        }
      });
  };
  const off = api.onPrPush((push) => {
    if (source.current.matches(push)) {
      read();
    }
  });
  read();
  return () => {
    live = false;
    off();
  };
}

/**
 * Reads `load` while `key` holds and `connected`, and again for each push
 * `matches` accepts. An answer for an older key is dropped.
 */
function useLiveRead<T>(
  api: PrApi,
  key: string | null,
  connected: boolean,
  load: () => Promise<T>,
  matches: (push: PrPush) => boolean,
): Loaded<T> {
  const [state, setState] = useState<Loaded<T> & { readonly key: string | null }>({
    data: null,
    error: null,
    key: null,
  });
  const source = useLatestRef<Source<T>>({ load, matches });

  useEffect(() => {
    if (key === null || !connected) {
      return undefined;
    }
    return follow(
      api,
      source,
      (data) => setState({ data, error: null, key }),
      (error) => setState((was) => ({ ...was, error: errText(error), key })),
    );
  }, [api, source, key, connected]);

  return state.key === key ? state : { data: null, error: null };
}

const EVERY_STATE = [PrState.OPEN, PrState.MERGING, PrState.MERGED, PrState.CLOSED];

/** Every pull request of a project, open first, then newest. */
export function usePrList(
  api: PrApi,
  projectId: string,
  connected: boolean,
): Loaded<PullRequest[]> {
  return useLiveRead(
    api,
    projectId,
    connected,
    () =>
      prCall(api, { case: "prList", value: { projectId, states: EVERY_STATE } }, "prList").then(
        (list) => list.prs,
      ),
    (push) => pushProject(push) === projectId,
  );
}

/** One pull request in full; read again on its `pr_updated`, a check on its head, or the queue. */
export function usePr(
  api: PrApi,
  projectId: string,
  number: number | null,
  connected: boolean,
): Loaded<PullRequest> {
  const head = useRef("");
  const loaded = useLiveRead(
    api,
    number === null ? null : `${projectId}#${number}`,
    connected,
    () => prCall(api, { case: "prGet", value: { projectId, number: number ?? 0 } }, "pr"),
    (push) => {
      if (pushProject(push) !== projectId) {
        return false;
      }
      switch (push.push.case) {
        case "prUpdated":
        case "cleanupUpdated":
          return push.push.value.number === number;
        case "checkUpdated":
          return push.push.value.sha === head.current;
        default:
          return true;
      }
    },
  );
  const headSha = loaded.data?.headSha ?? "";
  useEffect(() => {
    head.current = headSha;
  }, [headSha]);
  return loaded;
}

/** A PR's files and unified diff from `fromSha` (unset: from main) to its head. */
export function usePrDiff(
  api: PrApi,
  pr: PullRequest | null,
  fromSha: string | null,
  connected: boolean,
): Loaded<PrDiff> {
  const key = pr === null ? null : `${pr.projectId}#${pr.number}@${fromSha ?? ""}..${pr.headSha}`;
  return useLiveRead(
    api,
    key,
    connected,
    () =>
      prCall(
        api,
        {
          case: "prDiff",
          value: {
            projectId: pr?.projectId ?? "",
            number: pr?.number ?? 0,
            toSha: pr?.headSha,
            ...(fromSha === null ? {} : { fromSha }),
          },
        },
        "prDiff",
      ),
    () => false,
  );
}
