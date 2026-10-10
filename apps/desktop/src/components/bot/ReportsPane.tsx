// A bot's Reports tab, where its page opens (UX-024): what it is doing, the
// last thing it reported to the owner, and the questions it asked the owner
// that are still open, each with Reply.

import type { ReactElement } from "react";
import type { DaemonApi } from "../../protocol/api";
import type { Bot } from "../../protocol/entities";
import type { ThreadMessage } from "../../protocol/gen/hermes/home/v1/home_pb";
import { fromTheBot } from "../../protocol/home";
import { clock } from "../home/homeText";
import { useOwnerThread } from "../home/useOwnerThreads";
import { useProjectRow } from "../home/useProjectsOverview";
import { useNow } from "../control/useNow";

interface ReportsPaneProps {
  readonly client: DaemonApi;
  readonly bot: Bot;
  readonly connected: boolean;
  /** Answers the bot, quoting what it said. */
  readonly onReply: (quote: string) => void;
  /** Pins the clock, for stories and visual baselines. */
  readonly now?: number;
}

function Section(props: { readonly title: string; readonly children: ReactElement }): ReactElement {
  return (
    <section className="report-section">
      <h3>{props.title}</h3>
      {props.children}
    </section>
  );
}

function Message(props: {
  readonly message: ThreadMessage;
  readonly now: number;
  readonly onReply?: (quote: string) => void;
}): ReactElement {
  const { message, now } = props;
  return (
    <div className="report-message">
      <blockquote className="home-quote">
        {message.at === undefined ? null : <b>{clock(message.at, now)}: </b>}
        {message.text}
      </blockquote>
      {props.onReply === undefined ? null : (
        <button
          type="button"
          className="btn btn-small"
          onClick={() => {
            props.onReply?.(message.text);
          }}
        >
          Reply
        </button>
      )}
    </div>
  );
}

/** The newest message (oldest first in `messages`) that isn't an open question. */
function newestReport(messages: readonly ThreadMessage[]): ThreadMessage | undefined {
  for (let i = messages.length - 1; i >= 0; i -= 1) {
    const message = messages[i];
    if (message !== undefined && !(message.asks && message.open)) {
      return message;
    }
  }
  return undefined;
}

export default function ReportsPane(props: ReportsPaneProps): ReactElement {
  const { client, bot, connected } = props;
  const page = useOwnerThread(client, bot.id, connected);
  const row = useProjectRow(client, bot.project_id, connected);
  const clockNow = useNow();
  const now = props.now ?? clockNow;
  const doing = row?.doing.filter((d) => d.assigneeBotId === bot.id) ?? [];
  const fromBot = (page?.messages ?? []).filter(fromTheBot);
  const open = fromBot.filter((m) => m.asks && m.open);
  // Messages are oldest first. The latest report is the newest one that isn't
  // an open question: those are listed under "Asked you" alone (UX-027).
  const latest = newestReport(fromBot);
  // Only questions so far: the section is left out rather than empty.
  const showLatest = latest !== undefined || fromBot.length === 0;
  return (
    <div className="tab-pane tab-pane-scroll reports-pane">
      <Section title="Doing now">
        {doing.length === 0 ? (
          <p className="dash-empty">No board item in progress.</p>
        ) : (
          <ul className="home-doing">
            {doing.map((d) => (
              <li key={d.itemId}>
                <span className="mono">{d.itemId}</span> {d.title}
              </li>
            ))}
          </ul>
        )}
      </Section>
      {showLatest ? (
        <Section title="Latest report">
          {latest === undefined ? (
            <p className="dash-empty">Nothing reported yet.</p>
          ) : (
            <Message message={latest} now={now} onReply={props.onReply} />
          )}
        </Section>
      ) : null}
      <Section title={open.length > 0 ? `Asked you · ${open.length}` : "Asked you"}>
        {open.length === 0 ? (
          <p className="dash-empty">No open questions.</p>
        ) : (
          <>
            {open.map((m) => (
              <Message key={m.id} message={m} now={now} onReply={props.onReply} />
            ))}
          </>
        )}
      </Section>
    </div>
  );
}
