// A project's Team tab (UX-024, H-133 U2): its bots as cards, not a sidebar
// list. Each says its state in a glyph and a word, what it is doing, and the
// last thing it sent the owner; opening one shows its page, on Reports.

import type { ReactElement } from "react";
import type { Bot, BotState } from "../../protocol/entities";
import type { OwnerThread, ProjectRow } from "../../protocol/gen/hermes/home/v1/home_pb";
import BotAvatar from "../BotAvatar";
import { BOT_STATE_LABEL } from "../bot/botStates";
import { clock } from "../home/homeText";

type Tone = "ok" | "you" | "bad" | "off" | "wait";

const STATE_GLYPH: Readonly<Record<BotState, readonly [string, Tone]>> = {
  starting: ["◌", "wait"],
  ready: ["○", "off"],
  working: ["◑", "ok"],
  waiting_for_user: ["▲", "you"],
  waiting_for_approval: ["▲", "you"],
  rate_limited: ["⏸", "wait"],
  auth_failed: ["✗", "bad"],
  crashed: ["✗", "bad"],
  stopping: ["◌", "off"],
  stopped: ["○", "off"],
};

/** "◑ Working", in the state's tone. */
function StatePill({ state }: { readonly state: BotState }): ReactElement {
  const [glyph, tone] = STATE_GLYPH[state] ?? ["•", "wait"];
  return (
    <span className={`release-pill release-tone-${tone}`}>
      {glyph} {BOT_STATE_LABEL[state] ?? "Unknown"}
    </span>
  );
}

interface TeamViewProps {
  readonly bots: readonly Bot[];
  readonly row: ProjectRow | null;
  /** The owner's threads, for each bot's last word to the owner. */
  readonly threads: readonly OwnerThread[];
  readonly unread: Readonly<Record<string, number>>;
  readonly leadBotId: string | null | undefined;
  readonly now: number;
  readonly canControl: boolean;
  readonly onOpenBot: (botId: string) => void;
  readonly onCreateBot: () => void;
}

/** What the bot is doing: its board item, else its description. */
function doingLine(bot: Bot, row: ProjectRow | null, lead: boolean): string {
  const items = row?.doing.filter((d) => d.assigneeBotId === bot.id) ?? [];
  const doing = items.map((d) => `${d.itemId} ${d.title}`).join(" · ");
  const role = lead ? "Lead" : "";
  return [role, doing.length > 0 ? doing : bot.description].filter((s) => s.length > 0).join(" · ");
}

function BotCard(props: {
  readonly bot: Bot;
  readonly line: string;
  readonly thread: OwnerThread | undefined;
  readonly unread: number;
  readonly now: number;
  readonly onOpen: () => void;
}): ReactElement {
  const { bot, thread, now } = props;
  const last = thread?.last;
  const said = last !== undefined && !last.fromOwner ? last : undefined;
  return (
    <article className="team-card" aria-label={bot.name} data-testid="team-card">
      <header className="team-card-head">
        <BotAvatar avatar={bot.avatar} name={bot.name} id={bot.id} size="sm" />
        <h3>{bot.name}</h3>
        {props.unread > 0 ? (
          <span className="rail-badge team-unread" aria-label={`${props.unread} unread`}>
            {props.unread}
          </span>
        ) : null}
        <StatePill state={bot.state} />
      </header>
      <p className="team-line">{props.line.length > 0 ? props.line : "No current item"}</p>
      {said === undefined ? null : (
        <blockquote className="home-quote">
          <b>
            {said.asks && thread?.openQuestion ? "Asked you" : "Sent you"}
            {said.at === undefined ? "" : ` · ${clock(said.at, now)}`}:{" "}
          </b>
          “{said.text}”
        </blockquote>
      )}
      <button type="button" className="btn btn-small home-open" onClick={props.onOpen}>
        Open {bot.name}
      </button>
    </article>
  );
}

export default function TeamView(props: TeamViewProps): ReactElement {
  const working = props.bots.filter((b) => b.state === "working").length;
  return (
    <div className="home">
      <div className="home-scroll">
        <p className="team-summary">
          {props.bots.length === 1 ? "1 bot" : `${props.bots.length} bots`}
          {working > 0 ? ` · ${working} working` : ""}
          {props.canControl ? (
            <button type="button" className="btn btn-small" onClick={props.onCreateBot}>
              ＋ New bot
            </button>
          ) : null}
        </p>
        {props.bots.length === 0 ? (
          <p className="dash-empty">No bots in this project yet.</p>
        ) : (
          <div className="home-grid">
            {props.bots.map((bot) => (
              <BotCard
                key={bot.id}
                bot={bot}
                line={doingLine(bot, props.row, bot.id === props.leadBotId)}
                thread={props.threads.find((t) => t.bot?.botId === bot.id)}
                unread={props.unread[bot.id] ?? 0}
                now={props.now}
                onOpen={() => {
                  props.onOpenBot(bot.id);
                }}
              />
            ))}
          </div>
        )}
      </div>
    </div>
  );
}
