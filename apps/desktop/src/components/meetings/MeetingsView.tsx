// A project's Meetings tab (UX-024, H-133 U2): the series and recent meetings
// on the left, the open one's minutes on the right: the facilitator's summary,
// the outputs by section, and its action items. States pair a glyph and a word.

import type { ReactElement } from "react";
import type { DaemonApi } from "../../protocol/api";
import type { Bot, Project } from "../../protocol/entities";
import type { MeetingDetail, MeetingSummary } from "../../protocol/meetings";
import { OWNER } from "../../protocol/meetings";
import { when } from "../dashboard/needsYouText";
import { Widget } from "../dashboard/Widgets";
import { useMeetings } from "./useMeetings";

const STATUS: Readonly<Record<MeetingSummary["status"], string>> = {
  scheduled: "○ Scheduled",
  collecting: "◐ Collecting",
  held: "● Held",
  skipped: "⊘ Skipped",
};

/** "Stand-up · Mon 6 Oct". */
function heading(m: MeetingSummary): string {
  const at = m.closed_at ?? m.started_at;
  return at === null ? m.name : `${m.name} · ${when(at)}`;
}

function nameOf(bots: readonly Bot[], id: string): string {
  return id === OWNER ? "You" : (bots.find((b) => b.id === id)?.name ?? "a bot");
}

function Minutes(props: {
  readonly meeting: MeetingDetail;
  readonly bots: readonly Bot[];
}): ReactElement {
  const { meeting, bots } = props;
  const sections = Object.entries(meeting.outputs).filter(([, text]) => text.trim() !== "");
  const actions = [...meeting.action_items, ...meeting.carried_over];
  return (
    <>
      <p className="meeting-status">{STATUS[meeting.status]}</p>
      {meeting.summary.trim() === "" ? (
        <p className="dash-empty">
          {meeting.status === "collecting"
            ? `Collecting: ${meeting.contributed} of ${meeting.attendee_count} have contributed.`
            : "No summary yet."}
        </p>
      ) : (
        <blockquote className="home-quote meeting-summary">
          <b>Summary ({nameOf(bots, meeting.facilitator)}): </b>
          {meeting.summary}
        </blockquote>
      )}
      {sections.map(([section, text]) => (
        <section key={section} className="meeting-section">
          <h3>{section.replaceAll("_", " ")}</h3>
          <p>{text}</p>
        </section>
      ))}
      {actions.length === 0 ? null : (
        <section className="meeting-section">
          <h3>Action items</h3>
          <ul className="dash-rows">
            {actions.map((a) => (
              <li key={a.id} className="dash-row">
                <span className="dash-glyph" aria-hidden="true">
                  {a.status === "done" ? "☑" : a.status === "dropped" ? "⊘" : "☐"}
                </span>
                <span className="dash-row-text">
                  <span className="dash-row-title">{a.text}</span>
                  <span className="dash-row-meta">
                    {nameOf(bots, a.owner)}
                    {a.due_at === null ? "" : ` · due ${when(a.due_at)}`}
                    {a.item_id === null ? "" : ` · on the board as ${a.item_id}`}
                  </span>
                </span>
              </li>
            ))}
          </ul>
        </section>
      )}
    </>
  );
}

interface MeetingsViewProps {
  readonly client: DaemonApi;
  readonly project: Project;
  readonly bots: readonly Bot[];
  readonly connected: boolean;
}

export default function MeetingsView(props: MeetingsViewProps): ReactElement {
  const state = useMeetings(props.client, props.project.id, props.connected);
  if (state.error !== null) {
    return (
      <div className="empty-pane">
        <div className="empty-state" role="status">
          <p>{state.error}</p>
        </div>
      </div>
    );
  }
  const nothing = state.loaded && state.series.length === 0 && state.meetings.length === 0;
  return (
    <div className="dash">
      <div className="dash-scroll">
        <div className="dash-grid">
          <Widget id="meetings-list" title="Meetings">
            {nothing ? (
              <p className="dash-empty">
                No meetings yet. The lead sets up stand-ups, demos and retros.
              </p>
            ) : (
              <ul className="dash-rows">
                {state.series.map((s) => (
                  <li key={s.id} className="dash-row">
                    <span className="dash-glyph" aria-hidden="true">
                      ◷
                    </span>
                    <span className="dash-row-text">
                      <span className="dash-row-title">{s.name}</span>
                      <span className="dash-row-meta">
                        {s.next_at === null ? "⊘ Paused" : `○ Next: ${when(s.next_at)}`}
                      </span>
                    </span>
                  </li>
                ))}
                {state.meetings.map((m) => (
                  <li key={m.id} className="dash-row">
                    <span className="dash-glyph" aria-hidden="true">
                      {STATUS[m.status].slice(0, 1)}
                    </span>
                    <button
                      type="button"
                      className="dash-row-text dash-row-link"
                      aria-current={m.id === state.openId ? "true" : undefined}
                      onClick={() => {
                        state.select(m.id);
                      }}
                    >
                      <span className="dash-row-title">{heading(m)}</span>
                      <span className="dash-row-meta">{STATUS[m.status].slice(2)}</span>
                    </button>
                  </li>
                ))}
              </ul>
            )}
          </Widget>
          <Widget
            id="meetings-minutes"
            title={state.open === null ? "Minutes" : `${heading(state.open)} · minutes`}
          >
            {state.open === null ? (
              <p className="dash-empty">Pick a meeting to read its minutes.</p>
            ) : (
              <Minutes meeting={state.open} bots={props.bots} />
            )}
          </Widget>
        </div>
      </div>
    </div>
  );
}
