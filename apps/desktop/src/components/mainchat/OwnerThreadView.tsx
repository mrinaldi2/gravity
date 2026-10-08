// A bot's thread with the owner (H-132 D6): the one conversation the owner
// has with it (H-192), shown the same in the main chat and on the bot's own
// Chat tab, with one read marker.

import { useEffect, useRef } from "react";
import type { ReactElement, ReactNode } from "react";
import type { DaemonApi } from "../../protocol/api";
import type { ThreadMessage } from "../../protocol/gen/hermes/home/v1/home_pb";
import { useOwnerThread } from "../home/useOwnerThreads";
import { Bubble } from "./ChatParts";

/** Marks a thread read up to its newest message once the owner has it open. */
function useMarkRead(
  client: DaemonApi,
  botId: string,
  newest: bigint | undefined,
  lastRead: bigint | undefined,
): void {
  useEffect(() => {
    if (newest === undefined || (lastRead !== undefined && lastRead >= newest)) {
      return;
    }
    client
      .request(
        { type: "owner_thread_read", bot_id: botId, up_to_num: Number(newest) },
        "owner_thread_marked",
      )
      .catch(() => {
        // Read state is a convenience; the badge clears on the next read.
      });
  }, [client, botId, newest, lastRead]);
}

/** One bot's thread with the owner, oldest first, scrolled to the newest. */
export default function OwnerThreadView(props: {
  readonly client: DaemonApi;
  readonly botId: string;
  readonly connected: boolean;
  readonly now: number;
  /** Shown after the messages, given them: e.g. where an answer may be. */
  readonly after?: (messages: readonly ThreadMessage[]) => ReactNode;
}): ReactElement {
  const page = useOwnerThread(props.client, props.botId, props.connected);
  const end = useRef<HTMLDivElement | null>(null);
  const messages = page?.messages ?? [];
  useMarkRead(props.client, props.botId, messages.at(-1)?.num, page?.lastReadNum);
  const newest = messages.at(-1)?.id;
  useEffect(() => {
    // Each new message scrolls the thread to it.
    if (newest !== undefined) {
      end.current?.scrollIntoView?.({ block: "end" });
    }
  }, [newest]);
  return (
    <div className="mc-messages" aria-label="Messages">
      {messages.length === 0 ? (
        <p className="dash-empty">No messages yet. What you send shows here, and the answer too.</p>
      ) : (
        messages.map((m) => <Bubble key={m.id} message={m} now={props.now} />)
      )}
      {props.after?.(messages)}
      <div ref={end} />
    </div>
  );
}
