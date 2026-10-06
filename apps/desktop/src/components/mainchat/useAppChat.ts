// The main chat as the app shell holds it: the owner threads (read once for
// the panel and the rail's badge) and who "To" starts on from where the
// owner is: the bot on screen, else the open project's lead.

import { useCallback } from "react";
import { useLatestRef } from "../../app/useLatestRef";
import type { Selection } from "../../app/selection";
import type { DaemonApi } from "../../protocol/api";
import type { Project } from "../../protocol/entities";
import type { OwnerThread } from "../../protocol/gen/hermes/home/v1/home_pb";
import { useOwnerThreads } from "../home/useOwnerThreads";
import { useMainChat } from "./useMainChat";
import type { MainChatApi } from "./useMainChat";

export interface AppChat {
  readonly chat: MainChatApi;
  readonly threads: readonly OwnerThread[];
  /** Threads with a question waiting on the owner, for the rail's Chat badge. */
  readonly asking: number;
}

export function useAppChat(
  client: DaemonApi,
  connected: boolean,
  selection: Selection,
  projects: readonly Project[],
): AppChat {
  const threads = useOwnerThreads(client, connected);
  const selectionRef = useLatestRef(selection);
  const fallbackBot = useCallback((): string | null => {
    const current = selectionRef.current;
    if (current.kind === "bot") {
      return current.botId;
    }
    if (current.kind === "project") {
      return projects.find((p) => p.id === current.projectId)?.lead_bot_id ?? null;
    }
    return threads[0]?.bot?.botId ?? null;
  }, [projects, selectionRef, threads]);
  const chat = useMainChat(fallbackBot);
  return { chat, threads, asking: threads.filter((t) => t.openQuestion).length };
}
