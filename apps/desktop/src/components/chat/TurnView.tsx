import type { ReactElement } from "react";
import type { DaemonApi } from "../../protocol/api";
import type { ChatTurn } from "../../protocol/chat";
import { fmtTimestamp } from "../../util";
import LinkedText from "../cards/LinkedText";
import ChatMarkdown from "./ChatMarkdown";
import DecisionCard from "./DecisionCard";
import { durationLabel, groupItems, statsLine, triggerView } from "./chatModel";
import type { ChatBlock } from "./chatModel";
import StepGroup from "./StepGroup";

interface TurnViewProps {
  readonly client: DaemonApi;
  readonly turn: ChatTurn;
  /** Opens a file a result names in the Files panel. */
  readonly onOpenFile: (path: string) => void;
  readonly connected: boolean;
  readonly onOpenDecision?: (decisionId: string) => void;
  /** Opens every folded step group, e.g. while searching. */
  readonly expandAll?: boolean;
  /** Opens the bot's Chat, from a text tagged "In Chat" (H-192). */
  readonly onOpenChat?: () => void;
}

/** The text of the turn that was posted to the owner's Chat, if one was. */
function postedTextId(turn: ChatTurn): string | undefined {
  if (turn.answer_num === undefined) {
    return undefined;
  }
  for (let i = turn.items.length - 1; i >= 0; i -= 1) {
    const item = turn.items[i];
    if (item?.type === "text") {
      return item.id;
    }
  }
  return undefined;
}

/** One turn: what woke the bot, what it did, and how it ended. */
export default function TurnView(props: TurnViewProps): ReactElement {
  const { client, turn, onOpenFile } = props;
  const trigger = triggerView(turn.trigger);
  const posted = postedTextId(turn);
  const stats = statsLine(turn.stats);
  const meta = [fmtTimestamp(turn.started_at), durationLabel(turn.duration_ms)]
    .filter((part) => part !== "")
    .join(" · ");
  return (
    <article
      className={`chat-turn${turn.open ? " chat-turn-open" : ""}`}
      aria-label={trigger.label}
    >
      <header className={`chat-turn-head${trigger.own ? " chat-turn-head-own" : ""}`}>
        <span className="chat-turn-label">{trigger.label}</span>
        <span className="chat-turn-meta">{meta}</span>
      </header>
      {trigger.text === "" ? null : (
        <div className={trigger.own ? "chat-bubble chat-bubble-own" : "chat-bubble chat-bubble-in"}>
          {trigger.own ? (
            <LinkedText text={trigger.text} />
          ) : (
            <ChatMarkdown>{trigger.text}</ChatMarkdown>
          )}
        </div>
      )}
      {groupItems(turn.items).map((block) => (
        <Block
          key={block.kind === "steps" ? block.id : block.item.id}
          client={client}
          botId={turn.bot_id}
          turnOpen={turn.open}
          block={block}
          onOpenFile={onOpenFile}
          connected={props.connected}
          onOpenDecision={props.onOpenDecision}
          expandAll={props.expandAll}
          inChat={block.kind === "item" && block.item.id === posted}
          onOpenChat={props.onOpenChat}
        />
      ))}
      {turn.open ? <div className="chat-working">Working…</div> : null}
      {stats === "" || turn.open ? null : <footer className="chat-turn-stats">{stats}</footer>}
    </article>
  );
}

interface BlockProps {
  readonly client: DaemonApi;
  readonly botId: string;
  readonly turnOpen: boolean;
  readonly block: ChatBlock;
  readonly onOpenFile: (path: string) => void;
  readonly connected: boolean;
  readonly onOpenDecision?: (decisionId: string) => void;
  readonly expandAll?: boolean;
  /** This text is the answer posted to the owner's Chat. */
  readonly inChat?: boolean;
  readonly onOpenChat?: () => void;
}

/** The tag on an answer posted to the owner's Chat: a link there when it can be. */
function InChat({ onOpen }: { readonly onOpen?: () => void }): ReactElement {
  return onOpen === undefined ? (
    <span className="chat-in-chat">In Chat</span>
  ) : (
    <button type="button" className="chat-in-chat" onClick={onOpen}>
      In Chat
    </button>
  );
}

function Block(props: BlockProps): ReactElement {
  const { client, botId, turnOpen, block, onOpenFile } = props;
  if (block.kind === "steps") {
    return (
      <StepGroup
        client={client}
        botId={botId}
        steps={block.steps}
        turnOpen={turnOpen}
        expandAll={props.expandAll}
      />
    );
  }
  const { item } = block;
  switch (item.type) {
    case "text":
      return (
        <div className="chat-text">
          <ChatMarkdown>{item.markdown}</ChatMarkdown>
          {props.inChat === true ? <InChat onOpen={props.onOpenChat} /> : null}
        </div>
      );
    case "sent":
      // `message_owner`: a note in the owner's Chat (H-192).
      if (item.msg_kind === "owner") {
        return (
          <div className="chat-aside chat-sent chat-sent-owner">
            <span className="chat-aside-label">Sent you</span>
            <div className="chat-aside-body">{item.body}</div>
          </div>
        );
      }
      return (
        <div className="chat-aside chat-sent">
          <span className="chat-aside-label">
            To {item.to} · {item.msg_kind}
          </span>
          <div className="chat-aside-body">{item.body}</div>
        </div>
      );
    case "completed":
      return (
        <div className="chat-card chat-completed">
          <div className="chat-card-label">Task completed</div>
          <ChatMarkdown>{item.result}</ChatMarkdown>
          {item.artifacts.length === 0 ? null : (
            <div className="chat-files">
              {item.artifacts.map((file) => (
                <button
                  key={file.path}
                  type="button"
                  className="chat-file-chip"
                  title={file.path}
                  onClick={() => {
                    onOpenFile(file.path);
                  }}
                >
                  {file.name}
                </button>
              ))}
            </div>
          )}
        </div>
      );
    case "decision":
      return (
        <DecisionCard
          client={client}
          decisionId={item.decision_id}
          title={item.title}
          connected={props.connected}
          onOpenDecision={props.onOpenDecision}
        />
      );
    case "aside":
      return <div className={`chat-note chat-note-${item.kind}`}>{item.text}</div>;
    default:
      return item satisfies never;
  }
}
