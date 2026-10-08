// A bot's Chat tab (H-192, UX-033): the owner thread, the same conversation
// the main chat shows, with the bot page's composer. What the bot did, its
// tasks and its steps, is on Activity.

import { timestampDate } from "@bufbuild/protobuf/wkt";
import type { ReactElement, ReactNode } from "react";
import { nativeDictation } from "../../dictation";
import type { DaemonApi } from "../../protocol/api";
import type { Bot } from "../../protocol/entities";
import type { ThreadMessage } from "../../protocol/gen/hermes/home/v1/home_pb";
import ChatComposer from "../chat/ChatComposer";
import { uploadAttachment } from "../chat/upload";
import { useNow } from "../control/useNow";
import OwnerThreadView from "../mainchat/OwnerThreadView";

/** How long an unanswered message waits before the Activity hint shows. */
export const ANSWER_HINT_MS = 2 * 60 * 1000;

interface BotChatProps {
  readonly client: DaemonApi;
  readonly bot: Bot;
  readonly connected: boolean;
  /** Why the owner cannot write to this bot, or null when they can. */
  readonly writeBlocked: string | null;
  readonly onOpenActivity: () => void;
  /** Pins the clock, for stories and tests. */
  readonly now?: number;
}

/**
 * A service without the answer capture leaves a bot's answer in its
 * session: once the owner's latest message has waited, say where to look.
 */
export function unansweredSince(
  messages: readonly ThreadMessage[],
  now: number,
): ThreadMessage | null {
  const last = messages.at(-1);
  if (last?.fromOwner !== true || last.at === undefined) {
    return null;
  }
  return now - timestampDate(last.at).getTime() >= ANSWER_HINT_MS ? last : null;
}

export default function BotChat(props: BotChatProps): ReactElement {
  const { client, bot } = props;
  const clockNow = useNow();
  const now = props.now ?? clockNow;
  const captured = client.capabilities.includes("owner_answers");
  const linked = bot.peer != null;
  const after = (messages: readonly ThreadMessage[]): ReactNode =>
    captured || unansweredSince(messages, now) === null ? null : (
      <p className="chat-note bot-chat-hint">
        {bot.name} may have answered in Activity.{" "}
        <button type="button" className="chat-link" onClick={props.onOpenActivity}>
          See Activity
        </button>
      </p>
    );
  const send = async (text: string): Promise<void> => {
    // A failed send throws, so the composer keeps the draft.
    await client.request({ type: "send_user_message", to_bot_id: bot.id, body: text }, "message");
  };
  return (
    <div className="chat-pane bot-chat">
      <OwnerThreadView
        key={bot.id}
        client={client}
        botId={bot.id}
        connected={props.connected}
        now={now}
        after={after}
      />
      <ChatComposer
        disabledReason={props.writeBlocked}
        placeholder={`Message ${bot.name}`}
        onAttach={linked ? undefined : (file) => uploadAttachment(client, bot.project_id, file)}
        onSend={send}
        dictation={nativeDictation}
      />
    </div>
  );
}
