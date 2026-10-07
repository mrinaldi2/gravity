// The main chat's pieces (UX-024, H-133 U3): the threads, newest first; one
// thread's messages; and the composer with its To picker and the report it
// answers, quoted.

import { useEffect, useRef, useState } from "react";
import type { ReactElement } from "react";
import type { Bot, Project } from "../../protocol/entities";
import type { OwnerThread, ThreadMessage } from "../../protocol/gen/hermes/home/v1/home_pb";
import { sendOnCmdEnter } from "../../util";
import BotAvatar from "../BotAvatar";
import { clock } from "../home/homeText";

/** A thread's row: who, which project, when, and whether it waits on you. */
export function ThreadRow(props: {
  readonly thread: OwnerThread;
  readonly bot: Bot | undefined;
  readonly projectName: string;
  readonly active: boolean;
  readonly now: number;
  readonly onPick: () => void;
}): ReactElement {
  const { thread, now } = props;
  const name = props.bot?.name ?? thread.bot?.name ?? "A bot";
  const at = thread.last?.at;
  return (
    <button
      type="button"
      className={`mc-thread${props.active ? " mc-thread-on" : ""}`}
      aria-current={props.active ? "true" : undefined}
      onClick={props.onPick}
    >
      <span className="mc-thread-name">
        {name}
        {thread.openQuestion ? (
          <span className="mc-thread-asks" aria-label="asks you">
            ?
          </span>
        ) : null}
        {thread.unread > 0 ? (
          <span className="team-badge team-new" aria-label={`${thread.unread} new`}>
            {thread.unread}
          </span>
        ) : null}
      </span>
      <small>
        {props.projectName}
        {at === undefined ? "" : ` · ${clock(at, now)}`}
      </small>
    </button>
  );
}

/** One message: the owner's on the right, the bot's on the left. */
export function Bubble(props: {
  readonly message: ThreadMessage;
  readonly now: number;
}): ReactElement {
  const { message, now } = props;
  const mine = message.fromOwner;
  const label = message.asks && message.open ? "Asks you" : mine ? "You" : "";
  return (
    <div className={`mc-bubble${mine ? " mc-bubble-me" : ""}`}>
      <span className="mc-bubble-src">
        {label}
        {message.at === undefined ? "" : `${label ? " · " : ""}${clock(message.at, now)}`}
      </span>
      <span className="mc-bubble-text">{message.text}</span>
    </div>
  );
}

/** The To picker: every bot, by project, so a message reaches any of them. */
function ToPicker(props: {
  readonly bots: readonly Bot[];
  readonly projects: readonly Project[];
  readonly botId: string | null;
  readonly onPick: (botId: string) => void;
  readonly fresh: number;
}): ReactElement {
  const select = useRef<HTMLSelectElement | null>(null);
  useEffect(() => {
    // Each "New message" puts the owner here, to pick who it is for.
    if (props.fresh > 0) {
      select.current?.focus();
    }
  }, [props.fresh]);
  return (
    <label className="mc-to">
      To:
      <select
        ref={select}
        aria-label="To"
        value={props.botId ?? ""}
        onChange={(event) => {
          props.onPick(event.target.value);
        }}
      >
        {props.botId === null ? <option value="">Pick a bot…</option> : null}
        {props.projects.map((project) => (
          <optgroup key={project.id} label={project.name}>
            {props.bots
              .filter((bot) => bot.project_id === project.id)
              .map((bot) => (
                <option key={bot.id} value={bot.id}>
                  {bot.name}
                </option>
              ))}
          </optgroup>
        ))}
      </select>
    </label>
  );
}

/** "> " before each line of a quoted report, then the answer. */
export function withQuote(quote: string | null, text: string): string {
  if (quote === null) {
    return text;
  }
  const quoted = quote
    .split("\n")
    .map((line) => `> ${line}`)
    .join("\n");
  return `${quoted}\n\n${text}`;
}

export interface ComposerProps {
  readonly bots: readonly Bot[];
  readonly projects: readonly Project[];
  readonly botId: string | null;
  readonly quote: string | null;
  /** Why the owner can't send right now, or null. */
  readonly blocked: string | null;
  readonly onPick: (botId: string) => void;
  readonly onClearQuote: () => void;
  readonly onSend: (text: string) => Promise<boolean>;
  /** Counts "New message" presses: each focuses the To picker. */
  readonly fresh: number;
}

export function Composer(props: ComposerProps): ReactElement {
  const [text, setText] = useState("");
  const [sending, setSending] = useState(false);
  const canSend = props.botId !== null && props.blocked === null && text.trim() !== "" && !sending;
  const send = async (): Promise<void> => {
    if (!canSend) {
      return;
    }
    setSending(true);
    if (await props.onSend(text.trim())) {
      setText("");
    }
    setSending(false);
  };
  const onKeyDown = sendOnCmdEnter(send);
  return (
    <div className="mc-composer">
      <ToPicker
        bots={props.bots}
        projects={props.projects}
        botId={props.botId}
        onPick={props.onPick}
        fresh={props.fresh}
      />
      {props.quote === null ? null : (
        <div className="mc-quote">
          <span className="mc-quote-label">Replying to</span>
          <span className="mc-quote-text">{props.quote}</span>
          <button
            type="button"
            className="mc-quote-clear"
            aria-label="Don't quote it"
            onClick={props.onClearQuote}
          >
            ✕
          </button>
        </div>
      )}
      <textarea
        aria-label="Message"
        placeholder={props.blocked ?? "Message…"}
        disabled={props.blocked !== null}
        value={text}
        onChange={(event) => {
          setText(event.target.value);
        }}
        onKeyDown={onKeyDown}
      />
      <button
        type="button"
        className="btn btn-small btn-primary mc-send"
        disabled={!canSend}
        onClick={() => void send()}
      >
        Send ⌘↩
      </button>
    </div>
  );
}

export function BotHeading(props: {
  readonly bot: Bot | undefined;
  readonly projectName: string;
  readonly lead: boolean;
  readonly onOpenBot: (botId: string) => void;
}): ReactElement | null {
  const { bot } = props;
  if (bot === undefined) {
    return null;
  }
  return (
    <div className="mc-head">
      <BotAvatar avatar={bot.avatar} name={bot.name} id={bot.id} size="sm" />
      <b>{bot.name}</b>
      <span className="mc-head-project">
        {props.projectName}
        {props.lead ? " · lead" : ""}
      </span>
      <button
        type="button"
        className="btn btn-small mc-open-bot"
        onClick={() => {
          props.onOpenBot(bot.id);
        }}
      >
        Open bot
      </button>
    </div>
  );
}
