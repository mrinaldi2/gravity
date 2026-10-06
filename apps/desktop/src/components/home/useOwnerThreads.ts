// The owner's threads with the bots (H-132): one per bot across projects,
// read again whenever the service says one changed. Daemons before H-132
// have none, so the list stays empty there.

import { useCallback, useEffect, useState } from "react";
import { useLoadOnConnect } from "../../hooks/useLoadOnConnect";
import type { DaemonApi } from "../../protocol/api";
import type { OwnerThread, OwnerThreadPage } from "../../protocol/gen/hermes/home/v1/home_pb";
import { decodeOwnerThread, decodeOwnerThreads } from "../../protocol/home";

/** Whether the daemon keeps owner threads (H-132). */
function hasOwnerThreads(client: DaemonApi): boolean {
  return client.capabilities.includes("owner_threads");
}

export function useOwnerThreads(client: DaemonApi, connected: boolean): readonly OwnerThread[] {
  const supported = hasOwnerThreads(client);
  const [threads, setThreads] = useState<readonly OwnerThread[]>([]);
  const refresh = useCallback(async (): Promise<void> => {
    if (!supported) {
      return;
    }
    try {
      const reply = await client.request({ type: "owner_threads" }, "owner_threads");
      setThreads(decodeOwnerThreads(reply.owner_threads).threads);
    } catch {
      // Reports are a convenience here; the next push reads them again.
    }
  }, [client, supported]);
  useLoadOnConnect(connected, refresh);
  useEffect(() => {
    if (!supported) {
      return undefined;
    }
    return client.on("owner_thread_updated", () => {
      void refresh();
    });
  }, [client, refresh, supported]);
  return threads;
}

/** One bot's thread with the owner, newest page; read again when it changes. */
export function useOwnerThread(
  client: DaemonApi,
  botId: string,
  connected: boolean,
): OwnerThreadPage | null {
  const supported = hasOwnerThreads(client);
  const [page, setPage] = useState<OwnerThreadPage | null>(null);
  const refresh = useCallback(async (): Promise<void> => {
    if (!supported) {
      return;
    }
    try {
      const reply = await client.request(
        { type: "owner_thread_get", bot_id: botId, limit: 50 },
        "owner_thread",
      );
      setPage(decodeOwnerThread(reply.owner_thread));
    } catch {
      setPage(null);
    }
  }, [client, botId, supported]);
  useLoadOnConnect(connected, refresh);
  useEffect(() => {
    if (!supported) {
      return undefined;
    }
    return client.on("owner_thread_updated", (push) => {
      if ((push.bot?.bot_id ?? push.bot_id) === botId) {
        void refresh();
      }
    });
  }, [client, botId, refresh, supported]);
  return page;
}
