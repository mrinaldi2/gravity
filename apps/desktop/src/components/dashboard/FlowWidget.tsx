// The dashboard's Flow widget (B11, H-018 §2.1 "Flow"): how work moves,
// over this week or four. Items done with an 8-week sparkline, cycle time
// in plain words (typical, and what 85% finish within), rework and expired
// tasks as stats; time in each column, work in progress each day and the
// aging list below. Every chart is one series in the accent colour with its
// scale written under it, every bar names its value on hover, and a table
// view shows the same numbers as text (UX-019).

import { useState } from "react";
import type { ReactElement } from "react";
import type { ColumnTime, FlowDay, FlowMetrics, MetricsRange } from "../../protocol/metrics";
import CardLink from "../cards/CardLink";
import type { MetricsState } from "./useMetrics";

const DAY = 86_400;

/** "1.6d", "5h", "40m"; "—" when nothing was measured. */
export function duration(seconds: number, count = 1): string {
  if (count === 0) {
    return "—";
  }
  if (seconds >= DAY) {
    return `${(seconds / DAY).toFixed(1)}d`;
  }
  if (seconds >= 3600) {
    return `${Math.round(seconds / 3600)}h`;
  }
  return `${Math.max(1, Math.round(seconds / 60))}m`;
}

function unit(n: number, word: string): string {
  return `${n} ${word}${n === 1 ? "" : "s"}`;
}

/** "1.6 days", "5 hours", "40 minutes", for sentences. */
export function spelled(seconds: number): string {
  if (seconds >= DAY) {
    return `${(seconds / DAY).toFixed(1)} days`;
  }
  if (seconds >= 3600) {
    return unit(Math.round(seconds / 3600), "hour");
  }
  return unit(Math.max(1, Math.round(seconds / 60)), "minute");
}

function dayLabel(at: string): string {
  return new Date(at).toLocaleDateString([], { weekday: "short", day: "numeric", month: "short" });
}

function isEmpty(m: FlowMetrics): boolean {
  return m.throughput === 0 && m.weekly.every((n) => n === 0) && m.cycle.count === 0;
}

const RANGES: readonly { readonly key: MetricsRange; readonly label: string }[] = [
  { key: "week", label: "This week" },
  { key: "4w", label: "4 weeks" },
];

function Bars(props: {
  readonly label: string;
  readonly values: readonly {
    readonly key: string;
    readonly value: number;
    readonly title: string;
  }[];
}): ReactElement {
  const max = Math.max(1, ...props.values.map((v) => v.value));
  return (
    <ol className="flow-bars" aria-label={props.label}>
      {props.values.map((v) => (
        <li key={v.key} title={v.title} aria-label={v.title}>
          <span className="flow-bar" style={{ height: `${(v.value / max) * 100}%` }} />
        </li>
      ))}
    </ol>
  );
}

function CycleStat(props: { readonly m: FlowMetrics }): ReactElement {
  const { cycle } = props.m;
  return (
    <div className="flow-stat">
      <span className="flow-stat-label">Cycle time</span>
      {cycle.count === 0 ? (
        <span className="flow-stat-value">—</span>
      ) : (
        <>
          <span className="flow-stat-value">
            <strong>{spelled(cycle.p50)}</strong> typical
          </span>
          <span className="flow-stat-note">85% done within {spelled(cycle.p85)}</span>
        </>
      )}
    </div>
  );
}

function Stats(props: { readonly m: FlowMetrics }): ReactElement {
  const { m } = props;
  const weeks = m.weekly.map((n, i) => ({
    key: String(i),
    value: n,
    title: `${m.weekly.length - i === 1 ? "This week" : `${m.weekly.length - 1 - i} weeks ago`}: ${n} done`,
  }));
  return (
    <div className="flow-stats">
      <div className="flow-stat">
        <span className="flow-stat-label">Throughput</span>
        <span className="flow-stat-value">
          <strong>{m.throughput}</strong> {m.throughput === 1 ? "item" : "items"} done
        </span>
        <Bars label="Done per week, last 8 weeks" values={weeks} />
        <span className="flow-stat-note">Last 8 weeks</span>
      </div>
      <CycleStat m={m} />
      <div className="flow-stat">
        <span className="flow-stat-label">Rework</span>
        <span className="flow-stat-value">{Math.round(m.rework_rate * 100)}%</span>
        <span className="flow-stat-note">
          {m.reworked} of {m.past_doing} sent back
        </span>
      </div>
      <div className="flow-stat">
        <span className="flow-stat-label">Expired tasks</span>
        <span className="flow-stat-value">{m.expired_tasks}</span>
        <span className="flow-stat-note">tasks that ran out of time</span>
      </div>
    </div>
  );
}

/** "Doing: typical 1.2 days, 85% within 2.4 days, 6 items". */
function columnSentence(c: ColumnTime): string {
  if (c.count === 0) {
    return `${c.name}: nothing measured yet`;
  }
  const items = `${c.count} ${c.count === 1 ? "item" : "items"}`;
  return `${c.name}: typical ${spelled(c.p50)}, 85% within ${spelled(c.p85)}, ${items}`;
}

function ByColumn(props: { readonly columns: readonly ColumnTime[] }): ReactElement {
  const max = Math.max(1, ...props.columns.map((c) => c.p85));
  return (
    <ul className="flow-columns" aria-label="Time in each column">
      {props.columns.map((c) => (
        <li key={c.key} title={columnSentence(c)} aria-label={columnSentence(c)}>
          <span className="flow-column-name">{c.name}</span>
          <span className="flow-hbar">
            <span className="flow-bar" style={{ width: `${(c.p50 / max) * 100}%` }} />
          </span>
          <span className="flow-column-value">
            <strong>{duration(c.p50, c.count)}</strong> typical · 85% within{" "}
            {duration(c.p85, c.count)}
          </span>
        </li>
      ))}
    </ul>
  );
}

/** The bars, and their scale in words under them. */
function Wip(props: { readonly daily: readonly FlowDay[]; readonly days: number }): ReactElement {
  const now = props.daily.at(-1)?.wip ?? 0;
  const peak = Math.max(0, ...props.daily.map((d) => d.wip));
  return (
    <>
      <Bars
        label="In progress at the end of each day"
        values={props.daily.map((d) => ({
          key: d.at,
          value: d.wip,
          title: `${dayLabel(d.at)}: ${d.wip} in progress`,
        }))}
      />
      <span className="flow-stat-note">
        Now {now} · peak {peak} · {props.days === 7 ? "this week" : "last 4 weeks"}
      </span>
    </>
  );
}

function Tables(props: { readonly m: FlowMetrics }): ReactElement {
  const { m } = props;
  return (
    <div className="flow-tables">
      <table>
        <caption>Time in each column</caption>
        <thead>
          <tr>
            <th scope="col">Column</th>
            <th scope="col">Typical (p50)</th>
            <th scope="col">85% within (p85)</th>
            <th scope="col">Items</th>
          </tr>
        </thead>
        <tbody>
          {m.by_column.map((c) => (
            <tr key={c.key}>
              <th scope="row">{c.name}</th>
              <td>{duration(c.p50, c.count)}</td>
              <td>{duration(c.p85, c.count)}</td>
              <td>{c.count}</td>
            </tr>
          ))}
        </tbody>
      </table>
      <table>
        <caption>In progress by day</caption>
        <tbody>
          {m.daily.map((d) => (
            <tr key={d.at}>
              <th scope="row">{dayLabel(d.at)}</th>
              <td>{d.wip}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

function Charts(props: {
  readonly m: FlowMetrics;
  readonly columnName: (key: string) => string;
  readonly onBoard: () => void;
}): ReactElement {
  const { m } = props;
  return (
    <div className="flow-charts">
      <div>
        <h3>Time in each column</h3>
        <ByColumn columns={m.by_column} />
      </div>
      <div>
        <h3>In progress each day</h3>
        <Wip daily={m.daily} days={m.days} />
      </div>
      <div>
        <h3>Aging</h3>
        {m.aging.length === 0 ? (
          <p className="dash-empty">Nothing in progress.</p>
        ) : (
          <ul className="flow-aging">
            {m.aging.map((a) => (
              <li key={a.id}>
                <span className="mono">
                  <CardLink id={a.id} />
                </span>{" "}
                <button type="button" className="flow-link" onClick={props.onBoard}>
                  {a.title}
                </button>
                <span className="flow-stat-note">
                  {props.columnName(a.column_key)} {duration(a.age)}
                </span>
              </li>
            ))}
          </ul>
        )}
      </div>
    </div>
  );
}

function Body(props: {
  readonly state: MetricsState;
  readonly table: boolean;
  readonly columnName: (key: string) => string;
  readonly onBoard: () => void;
}): ReactElement {
  const { metrics, note, loaded, error } = props.state;
  if (metrics === null) {
    let text = loaded ? "No board yet. Start it from the Board tab." : "Loading…";
    if (note) {
      text = note;
    } else if (error) {
      text = `Couldn't load the flow: ${error}`;
    }
    return <p className="dash-empty">{text}</p>;
  }
  if (isEmpty(metrics)) {
    return <p className="dash-empty">Metrics appear after the first item reaches Done.</p>;
  }
  return (
    <>
      <Stats m={metrics} />
      {props.table ? (
        <Tables m={metrics} />
      ) : (
        <Charts m={metrics} columnName={props.columnName} onBoard={props.onBoard} />
      )}
    </>
  );
}

/** The range and Table view toggles: only when there are numbers to show. */
function Toggles(props: {
  readonly range: MetricsRange;
  readonly setRange: (range: MetricsRange) => void;
  readonly table: boolean;
  readonly setTable: (table: boolean) => void;
}): ReactElement {
  return (
    <span className="flow-controls">
      <span className="flow-range" role="group" aria-label="Range">
        {RANGES.map((r) => (
          <button
            key={r.key}
            type="button"
            aria-pressed={props.range === r.key}
            onClick={() => props.setRange(r.key)}
          >
            {r.label}
          </button>
        ))}
      </span>
      <button
        type="button"
        className="dash-more"
        aria-pressed={props.table}
        onClick={() => props.setTable(!props.table)}
      >
        {props.table ? "Chart view" : "Table view"}
      </button>
    </span>
  );
}

export default function FlowWidget(props: {
  readonly state: MetricsState;
  readonly columnName: (key: string) => string;
  readonly onBoard: () => void;
}): ReactElement {
  const [table, setTable] = useState(false);
  const { range, setRange, metrics } = props.state;
  // They do nothing without numbers (UX-019).
  const toggles = metrics !== null && !isEmpty(metrics);
  return (
    <section className="dash-widget dash-wide" aria-labelledby="dash-flow">
      <h2 id="dash-flow">
        Flow
        {toggles ? (
          <Toggles range={range} setRange={setRange} table={table} setTable={setTable} />
        ) : null}
      </h2>
      <Body
        state={props.state}
        table={table}
        columnName={props.columnName}
        onBoard={props.onBoard}
      />
    </section>
  );
}
