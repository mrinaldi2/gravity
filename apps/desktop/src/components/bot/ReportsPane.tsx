// A bot's Reports tab, where its page opens (UX-024): what it is doing, the
// last thing it reported to the owner, and the questions it asked the owner
// that are still open, each with Reply.

import type { ReactElement } from "react";
import type { DaemonApi } from "../../protocol/api";
import type { Bot } from "../../protocol/entities";
import type { ThreadMessage } from "../../protocol/gen/hermes/home/v1/home_pb";
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

export default function ReportsPane(props: ReportsPaneProps): ReactElement {
  const { client, bot, connected } = props;
  const page = useOwnerThread(client, bot.id, connected);
  const row = useProjectRow(client, bot.project_id, connected);
  const clockNow = useNow();
  const now = props.now ?? clockNow;
  const doing = row?.doing.filter((d) => d.assigneeBotId === bot.id) ?? [];
  const fromBot = (page?.messages ?? []).filter((m) => !m.fromOwner);
  // Messages are oldest first; the latest report is the last one the bot sent.
  const latest = fromBot.at(-1);
  const open = fromBot.filter((m) => m.asks && m.open);
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
      <Section title="Latest report">
        {latest === undefined ? (
          <p className="dash-empty">{bot.name} hasn't sent you anything yet.</p>
        ) : (
          <Message message={latest} now={now} onReply={props.onReply} />
        )}
      </Section>
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
