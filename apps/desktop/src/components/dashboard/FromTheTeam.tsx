// "From the team" on a project's Overview (UX-024 §4): the lead's latest
// summary and what each bot last sent the owner, newest first, each with a
// Reply that opens a message to that bot quoting it.

import { timestampDate } from "@bufbuild/protobuf/wkt";
import type { Timestamp } from "@bufbuild/protobuf/wkt";
import type { ReactElement } from "react";
import type { Bot } from "../../protocol/entities";
import type { OwnerThread, ProjectRow } from "../../protocol/gen/hermes/home/v1/home_pb";
import BotAvatar from "../BotAvatar";
import { clock } from "../home/homeText";
import { Widget } from "./Widgets";

/** Opens a message to a bot, quoting what it said. */
export type ReplyTo = (botId: string, quote: string) => void;

interface Report {
  readonly key: string;
  readonly bot: Bot | undefined;
  readonly who: string;
  readonly what: string;
  readonly text: string;
  readonly at: Timestamp | undefined;
  readonly asks: boolean;
  readonly meeting: boolean;
}

/** The reports to show: the summary, then each bot's last word to the owner. */
function reports(
  row: ProjectRow | null,
  threads: readonly OwnerThread[],
  bots: readonly Bot[],
  lead: Bot | undefined,
): Report[] {
  const out: Report[] = [];
  const summary = row?.latestSummary;
  if (summary !== undefined && summary.text.length > 0) {
    out.push({
      key: `summary:${summary.meetingId}`,
      bot: lead,
      who: lead?.name ?? "Lead",
      what: "meeting summary",
      text: summary.text,
      at: summary.at,
      asks: false,
      meeting: true,
    });
  }
  for (const thread of threads) {
    const last = thread.last;
    if (last === undefined || last.fromOwner) {
      continue;
    }
    const bot = bots.find((b) => b.id === thread.bot?.botId);
    out.push({
      key: `thread:${thread.bot?.daemonId}:${thread.bot?.botId}`,
      bot,
      who: bot?.name ?? thread.bot?.name ?? "A bot",
      what: last.asks && thread.openQuestion ? "asks you" : "message",
      text: last.text,
      at: last.at,
      asks: last.asks && thread.openQuestion,
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
}

function ReportRow(props: {
  readonly report: Report;
  readonly now: number;
  readonly onReply: ReplyTo;
  readonly onOpenMeetings: () => void;
}): ReactElement {
  const { report, now } = props;
  const bot = report.bot;
  return (
    <li className="dash-row team-report">
      {bot === undefined ? (
        <span className="dash-glyph" aria-hidden="true">
          ●
        </span>
      ) : (
        <BotAvatar avatar={bot.avatar} name={bot.name} id={bot.id} size="sm" />
      )}
      <div className="dash-row-text">
        <span className="dash-row-title">
          {report.who} · {report.what}
          {report.at === undefined ? "" : ` · ${clock(report.at, now)}`}
        </span>
        <span className="team-report-text">{report.text}</span>
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
  const list = reports(props.row, props.threads, props.bots, lead);
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
