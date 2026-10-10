// "From the team" on a project's Overview (UX-024 §4): the lead's latest
// summary and what each bot last sent the owner, newest first, each with a
// Reply that opens a message to that bot quoting it.

import { timestampDate } from "@bufbuild/protobuf/wkt";
import type { Timestamp } from "@bufbuild/protobuf/wkt";
import type { ReactElement } from "react";
import type { Bot } from "../../protocol/entities";
import { fromTheBot } from "../../protocol/home";
import type {
  OwnerThread,
  ProjectRow,
  SummaryBrief,
} from "../../protocol/gen/hermes/home/v1/home_pb";
import type { MeetingSummary } from "../../protocol/meetings";
import { OWNER } from "../../protocol/meetings";
import BotAvatar from "../BotAvatar";
import LinkedText from "../cards/LinkedText";
import { clock } from "../home/homeText";
import { minutesTitle } from "../meetings/useMeetings";
import { Widget } from "./Widgets";

/** Opens a message to a bot, quoting what it said. */
export type ReplyTo = (botId: string, quote: string) => void;

interface Report {
  readonly key: string;
  /** Who Reply answers. */
  readonly bot: Bot | undefined;
  /** Whose avatar shows: who said it, when that is known. */
  readonly face: Bot | undefined;
  /** "Desktop Dev · asks you", "Stand-up minutes · Scrum Master". */
  readonly title: string;
  /** Names the Reply button: "Reply to Desktop Dev". */
  readonly who: string;
  readonly text: string;
  readonly at: Timestamp | undefined;
  readonly asks: boolean;
  readonly meeting: boolean;
}

/**
 * The latest summary, named by its meeting and signed by its facilitator, as
 * the Meetings tab signs it (UX-027). Until the meeting loads (it may never,
 * on an older service or off-home) it is "Meeting minutes · <time>", signed
 * by no one: the lead didn't necessarily write it (H-157). Reply still
 * reaches the lead then.
 */
function summaryReport(
  summary: SummaryBrief,
  meeting: MeetingSummary | null,
  bots: readonly Bot[],
  lead: Bot | undefined,
): Report {
  if (meeting === null) {
    return {
      key: `summary:${summary.meetingId}`,
      bot: lead,
      face: undefined,
      title: "Meeting minutes",
      who: lead?.name ?? "Lead",
      text: summary.text,
      at: summary.at,
      asks: false,
      meeting: true,
    };
  }
  const facilitator = bots.find((b) => b.id === meeting.facilitator);
  const bot = facilitator ?? lead;
  const signed =
    meeting.facilitator === OWNER ? "You" : (facilitator?.name ?? lead?.name ?? "Lead");
  return {
    key: `summary:${summary.meetingId}`,
    bot,
    face: bot,
    title: `${minutesTitle(meeting)} · ${signed}`,
    who: bot?.name ?? signed,
    text: summary.text,
    at: summary.at,
    asks: false,
    meeting: true,
  };
}

/** The reports to show: the summary, then each bot's last word to the owner. */
function reports(props: FromTheTeamProps, lead: Bot | undefined): Report[] {
  const { bots } = props;
  const out: Report[] = [];
  const summary = props.row?.latestSummary;
  if (summary !== undefined && summary.text.length > 0) {
    out.push(summaryReport(summary, props.summaryMeeting ?? null, bots, lead));
  }
  for (const thread of props.threads) {
    const last = thread.last;
    if (last === undefined || !fromTheBot(last)) {
      continue;
    }
    const bot = bots.find((b) => b.id === thread.bot?.botId);
    const who = bot?.name ?? thread.bot?.name ?? "A bot";
    const asks = last.asks && thread.openQuestion;
    out.push({
      key: `thread:${thread.bot?.daemonId}:${thread.bot?.botId}`,
      bot,
      face: bot,
      title: `${who} · ${asks ? "asks you" : "sent you"}`,
      who,
      text: last.text,
      at: last.at,
      asks,
      meeting: false,
    });
  }
  // oxlint-disable-next-line unicorn/no-array-sort -- `out` is ours; the lib has no toSorted.
  return out.sort((a, b) => Number(b.asks) - Number(a.asks) || ms(b.at) - ms(a.at)).slice(0, 6);
}

function ms(at: Timestamp | undefined): number {
  return at === undefined ? 0 : timestampDate(at).getTime();
}

export interface FromTheTeamProps {
  readonly row: ProjectRow | null;
  /** The owner's threads with this project's bots. */
  readonly threads: readonly OwnerThread[];
  readonly bots: readonly Bot[];
  readonly leadBotId: string | null | undefined;
  readonly now: number;
  readonly onReply: ReplyTo;
  readonly onOpenMeetings: () => void;
  /** The meeting the latest summary came from, to name and sign it. */
  readonly summaryMeeting?: MeetingSummary | null;
}

function ReportRow(props: {
  readonly report: Report;
  readonly now: number;
  readonly onReply: ReplyTo;
  readonly onOpenMeetings: () => void;
}): ReactElement {
  const { report, now } = props;
  const { bot, face } = report;
  return (
    <li className="dash-row team-report">
      {face === undefined ? (
        <span className="dash-glyph" aria-hidden="true">
          ●
        </span>
      ) : (
        <BotAvatar avatar={face.avatar} name={face.name} id={face.id} size="sm" />
      )}
      <div className="dash-row-text">
        <span className="dash-row-title">
          {report.title}
          {report.at === undefined ? "" : ` · ${clock(report.at, now)}`}
        </span>
        <span className="team-report-text">
          <LinkedText text={report.text} />
        </span>
      </div>
      <div className="team-report-actions">
        {report.meeting ? (
          <button type="button" className="btn btn-small" onClick={props.onOpenMeetings}>
            Open
          </button>
        ) : null}
        {bot === undefined ? null : (
          <button
            type="button"
            className="btn btn-small"
            aria-label={`Reply to ${report.who}`}
            onClick={() => {
              props.onReply(bot.id, report.text);
            }}
          >
            Reply
          </button>
        )}
      </div>
    </li>
  );
}

export default function FromTheTeam(props: FromTheTeamProps): ReactElement {
  const lead = props.bots.find((b) => b.id === props.leadBotId);
  const list = reports(props, lead);
  return (
    <Widget id="dash-from-team" title="From the team">
      {list.length === 0 ? (
        <p className="dash-empty">
          Nothing from the team yet. Summaries and messages the bots send you show here.
        </p>
      ) : (
        <ul className="dash-rows">
          {list.map((report) => (
            <ReportRow
              key={report.key}
              report={report}
              now={props.now}
              onReply={props.onReply}
              onOpenMeetings={props.onOpenMeetings}
            />
          ))}
        </ul>
      )}
    </Widget>
  );
}
