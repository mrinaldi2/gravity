// The main chat (UX-024 §2, H-133 U3): a right-hand panel, opened with ⌘J
// from anywhere, with a thread per bot across projects (the owner threads,
// H-132) and a composer that reaches any bot. Reply on a report opens it on
// that bot with the report quoted.

import { useEffect, useRef } from "react";
import type { ReactElement } from "react";
import type { AddToast } from "../../app/useToasts";
import type { DaemonApi } from "../../protocol/api";
import type { Bot, Project } from "../../protocol/entities";
import type { OwnerThread } from "../../protocol/gen/hermes/home/v1/home_pb";
import { errText } from "../../util";
import { useNow } from "../control/useNow";
import { useOwnerThread } from "../home/useOwnerThreads";
import { BotHeading, Bubble, Composer, ThreadRow, withQuote } from "./ChatParts";
import type { MainChatApi } from "./useMainChat";

interface MainChatProps {
  readonly client: DaemonApi;
  readonly connected: boolean;
  readonly canControl: boolean;
  readonly bots: readonly Bot[];
  readonly projects: readonly Project[];
  readonly threads: readonly OwnerThread[];
  readonly chat: MainChatApi;
  readonly addToast: AddToast;
  readonly onOpenBot: (botId: string) => void;
  /** Pins the clock, for stories and visual baselines. */
  readonly now?: number;
}

function projectName(projects: readonly Project[], id: string | undefined): string {
  return projects.find((p) => p.id === id)?.name ?? "";
}

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
function Thread(props: {
  readonly client: DaemonApi;
  readonly botId: string;
  readonly connected: boolean;
  readonly now: number;
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
      <div ref={end} />
    </div>
  );
}

function blockedReason(connected: boolean, canControl: boolean): string | null {
  if (!connected) {
    return "Not connected to the Hermes service";
  }
  return canControl ? null : "Read-only connection";
}

export default function MainChat(props: MainChatProps): ReactElement | null {
  const { client, chat, bots, projects } = props;
  const clockNow = useNow();
  const now = props.now ?? clockNow;
  if (!chat.open) {
    return null;
  }
  const bot = bots.find((b) => b.id === chat.botId);
  const project = projects.find((p) => p.id === bot?.project_id);
  const send = async (text: string): Promise<boolean> => {
    if (chat.botId === null) {
      return false;
    }
    try {
      await client.request(
        { type: "send_user_message", to_bot_id: chat.botId, body: withQuote(chat.quote, text) },
        "message",
      );
      chat.clearQuote();
      return true;
    } catch (failure) {
      props.addToast("error", "Couldn't send the message", errText(failure));
      return false;
    }
  };
  return (
    <aside className="main-chat" aria-label="Chat">
      <nav className="mc-threads" aria-label="Threads">
        <div className="mc-threads-head">
          <b>Chat</b>
          <button
            type="button"
            className="mc-close"
            aria-label="Close chat (⌘J)"
            title="Close (⌘J)"
            onClick={chat.close}
          >
            ✕
          </button>
        </div>
        {props.threads.map((thread) => {
          const id = thread.bot?.botId ?? "";
          return (
            <ThreadRow
              key={`${thread.bot?.daemonId}:${id}`}
              thread={thread}
              bot={bots.find((b) => b.id === id)}
              projectName={projectName(projects, thread.projectId)}
              active={id === chat.botId}
              now={now}
              onPick={() => {
                chat.pick(id);
              }}
            />
          );
        })}
        {props.threads.length === 0 ? (
          <p className="dash-empty">No threads yet. Pick a bot below to write to it.</p>
        ) : null}
      </nav>
      <section className="mc-thread-view">
        <BotHeading
          bot={bot}
          projectName={project?.name ?? ""}
          lead={project?.lead_bot_id === bot?.id}
          onOpenBot={props.onOpenBot}
        />
        {chat.botId === null ? (
          <p className="dash-empty">Pick who to write to.</p>
        ) : (
          <Thread
            key={chat.botId}
            client={client}
            botId={chat.botId}
            connected={props.connected}
            now={now}
          />
        )}
        <Composer
          bots={bots}
          projects={projects}
          botId={chat.botId}
          quote={chat.quote}
          blocked={blockedReason(props.connected, props.canControl)}
          onPick={chat.pick}
          onClearQuote={chat.clearQuote}
          onSend={send}
        />
      </section>
    </aside>
  );
}
