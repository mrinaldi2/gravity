// A project's Team tab (UX-024, H-133 U2): its bots as cards, in place of the
// old sidebar list. Each says its state in a glyph and a word, what it is
// doing, the last thing it sent the owner, and what is new; opening one shows
// its page, on Reports. ⋯ holds what the sidebar row's menu did.

import type { ReactElement } from "react";
import type { Bot, BotActivity, BotState } from "../../protocol/entities";
import type { OwnerThread, ProjectRow } from "../../protocol/gen/hermes/home/v1/home_pb";
import BotAvatar from "../BotAvatar";
import { BOT_STATE_LABEL } from "../bot/botStates";
import { clock } from "../home/homeText";
import { useRowMenu } from "../sidebar/useRowMenu";

type Tone = "acc" | "ok" | "you" | "bad" | "off" | "wait";

/** The app's state colours: Working in the accent, Idle in green (UX-027). */
const STATE_GLYPH: Readonly<Record<BotState, readonly [string, Tone]>> = {
  starting: ["◌", "wait"],
  ready: ["○", "ok"],
  working: ["◑", "acc"],
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

/** What changed for a bot since the owner last looked. */
interface Signals {
  readonly unread: number;
  readonly failed: number;
  /** Its last word anywhere, shown when it sent the owner nothing. */
  readonly activity: BotActivity | undefined;
}

interface TeamViewProps {
  readonly bots: readonly Bot[];
  readonly row: ProjectRow | null;
  /** The owner's threads, for each bot's last word to the owner. */
  readonly threads: readonly OwnerThread[];
  readonly unread: Readonly<Record<string, number>>;
  readonly failed: Readonly<Record<string, number>>;
  readonly activity: Readonly<Record<string, BotActivity>>;
  readonly leadBotId: string | null | undefined;
  readonly now: number;
  readonly canControl: boolean;
  readonly onOpenBot: (botId: string) => void;
  readonly onCreateBot: () => void;
  readonly onDeleteBot: (botId: string) => void;
}

/** ⋯ on a card; deleting is confirmed first. */
function CardMenu(props: {
  readonly bot: Bot;
  readonly canControl: boolean;
  readonly onDelete: () => void;
}): ReactElement | null {
  const menu = useRowMenu({
    items: [],
    deletion: props.canControl
      ? {
          label: "Delete bot",
          title: `Delete ${props.bot.name}?`,
          body: "Its session is stopped and the bot is archived along with its conversations.",
          confirmLabel: "Delete bot",
          onConfirm: props.onDelete,
        }
      : null,
  });
  if (!props.canControl) {
    return null;
  }
  return (
    <>
      <button
        type="button"
        className="team-menu"
        aria-label={`More for ${props.bot.name}`}
        onClick={menu.onOpenFrom}
      >
        ⋯
      </button>
      {menu.overlays}
    </>
  );
}

/** What the bot is doing: its board item, else its description. */
function doingLine(bot: Bot, row: ProjectRow | null, lead: boolean): string {
  const items = row?.doing.filter((d) => d.assigneeBotId === bot.id) ?? [];
  const doing = items.map((d) => `${d.itemId} ${d.title}`).join(" · ");
  const role = lead ? "Lead" : "";
  return [role, doing.length > 0 ? doing : bot.description].filter((s) => s.length > 0).join(" · ");
}

/** "1 new", "⚠ 2 failed deliveries": badges in words. */
function Badges({ signals }: { readonly signals: Signals }): ReactElement {
  const { unread, failed } = signals;
  return (
    <>
      {unread > 0 ? (
        <span
          className="team-badge team-new"
          aria-label={`${unread} new ${unread === 1 ? "message" : "messages"}`}
        >
          {unread} new
        </span>
      ) : null}
      {failed > 0 ? (
        <span
          className="team-badge team-failed"
          title={`${failed} failed ${failed === 1 ? "delivery" : "deliveries"}`}
        >
          ⚠ {failed}
        </span>
      ) : null}
    </>
  );
}

/** The last thing the bot sent the owner, else the last thing it said at all. */
function LastWord(props: {
  readonly thread: OwnerThread | undefined;
  readonly activity: BotActivity | undefined;
  readonly now: number;
}): ReactElement | null {
  const last = props.thread?.last;
  if (last !== undefined && !last.fromOwner) {
    const asked = last.asks && props.thread?.openQuestion === true;
    return (
      <blockquote className="home-quote">
        <b>
          {asked ? "Asked you" : "Sent you"}
          {last.at === undefined ? "" : ` · ${clock(last.at, props.now)}`}:{" "}
        </b>
        “{last.text}”
      </blockquote>
    );
  }
  const said = props.activity;
  return said === undefined ? null : (
    <p className="team-line team-activity" title={said.at}>
      {said.text}
    </p>
  );
}

function BotCard(props: {
  readonly bot: Bot;
  readonly line: string;
  readonly thread: OwnerThread | undefined;
  readonly signals: Signals;
  readonly now: number;
  readonly canControl: boolean;
  readonly onOpen: () => void;
  readonly onDelete: () => void;
}): ReactElement {
  const { bot } = props;
  return (
    <article className="team-card" aria-label={bot.name} data-testid="team-card">
      <header className="team-card-head">
        <BotAvatar avatar={bot.avatar} name={bot.name} id={bot.id} size="sm" />
        <h3>{bot.name}</h3>
        <Badges signals={props.signals} />
        <StatePill state={bot.state} />
        <CardMenu bot={bot} canControl={props.canControl} onDelete={props.onDelete} />
      </header>
      <p className="team-line">{props.line.length > 0 ? props.line : "No current item"}</p>
      <LastWord thread={props.thread} activity={props.signals.activity} now={props.now} />
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
                signals={{
                  unread: props.unread[bot.id] ?? 0,
                  failed: props.failed[bot.id] ?? 0,
                  activity: props.activity[bot.id],
                }}
                now={props.now}
                canControl={props.canControl}
                onOpen={() => {
                  props.onOpenBot(bot.id);
                }}
                onDelete={() => {
                  props.onDeleteBot(bot.id);
                }}
              />
            ))}
          </div>
        )}
      </div>
    </div>
  );
}
