// The dashboard's Flow widget (B11, H-018 §2.1 "Flow"): how work moves,
// over this week or four. Throughput with an 8-week sparkline, cycle time
// p50/p85, rework and expired tasks as stats; cycle time by column, WIP over
// time and the aging list below. Every chart is one series in the accent
// colour, every bar names its value on hover, and a table view shows the
// same numbers as text.

import { useState } from "react";
import type { ReactElement } from "react";
import type { ColumnTime, FlowDay, FlowMetrics, MetricsRange } from "../../protocol/metrics";
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
        <span className="flow-stat-value">{m.throughput}</span>
        <Bars label="Done per week, last 8 weeks" values={weeks} />
      </div>
      <div className="flow-stat">
        <span className="flow-stat-label">Cycle time</span>
        <span className="flow-stat-value">
          p50 {duration(m.cycle.p50, m.cycle.count)} · p85 {duration(m.cycle.p85, m.cycle.count)}
        </span>
      </div>
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
      </div>
    </div>
  );
}

function ByColumn(props: { readonly columns: readonly ColumnTime[] }): ReactElement {
  const max = Math.max(1, ...props.columns.map((c) => c.p85));
  return (
    <ul className="flow-columns" aria-label="Cycle time by column">
      {props.columns.map((c) => (
        <li
          key={c.key}
          title={`${c.name}: p50 ${duration(c.p50, c.count)}, p85 ${duration(c.p85, c.count)}`}
        >
          <span className="flow-column-name">{c.name}</span>
          <span className="flow-hbar">
            <span className="flow-bar" style={{ width: `${(c.p50 / max) * 100}%` }} />
          </span>
          <span className="flow-column-value">
            {duration(c.p50, c.count)} · p85 {duration(c.p85, c.count)}
          </span>
        </li>
      ))}
    </ul>
  );
}

function Wip(props: { readonly daily: readonly FlowDay[] }): ReactElement {
  return (
    <Bars
      label="In progress at the end of each day"
      values={props.daily.map((d) => ({
        key: d.at,
        value: d.wip,
        title: `${dayLabel(d.at)}: ${d.wip} in progress`,
      }))}
    />
  );
}

function Tables(props: { readonly m: FlowMetrics }): ReactElement {
  const { m } = props;
  return (
    <div className="flow-tables">
      <table>
        <caption>Cycle time by column</caption>
        <thead>
          <tr>
            <th scope="col">Column</th>
            <th scope="col">p50</th>
            <th scope="col">p85</th>
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
        <h3>Cycle time by column</h3>
        <ByColumn columns={m.by_column} />
      </div>
      <div>
        <h3>In progress over time</h3>
        <Wip daily={m.daily} />
      </div>
      <div>
        <h3>Aging</h3>
        {m.aging.length === 0 ? (
          <p className="dash-empty">Nothing in progress.</p>
        ) : (
          <ul className="flow-aging">
            {m.aging.map((a) => (
              <li key={a.id}>
                <button type="button" className="flow-link" onClick={props.onBoard}>
                  <span className="mono">{a.id}</span> {a.title}
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

export default function FlowWidget(props: {
  readonly state: MetricsState;
  readonly columnName: (key: string) => string;
  readonly onBoard: () => void;
}): ReactElement {
  const [table, setTable] = useState(false);
  const { range, setRange } = props.state;
  return (
    <section className="dash-widget dash-wide" aria-labelledby="dash-flow">
      <h2 id="dash-flow">
        Flow
        <span className="flow-controls">
          <span className="flow-range" role="group" aria-label="Range">
            {RANGES.map((r) => (
              <button
                key={r.key}
                type="button"
                aria-pressed={range === r.key}
                onClick={() => setRange(r.key)}
              >
                {r.label}
              </button>
            ))}
          </span>
          <button
            type="button"
            className="dash-more"
            aria-pressed={table}
            onClick={() => setTable((t) => !t)}
          >
            {table ? "Chart view" : "Table view"}
          </button>
        </span>
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
