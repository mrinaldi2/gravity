import type { Story } from "@ladle/react";
import type { ReactElement } from "react";
import type { FlowMetrics, MetricsRange } from "../../protocol/metrics";
import { emptyFlow, flowMetrics, flowMetrics4w } from "../../test/flowFixtures";
import FlowWidget from "./FlowWidget";
import type { MetricsState } from "./useMetrics";

const names: Readonly<Record<string, string>> = {
  doing: "Doing",
  review: "Review",
  verify: "Verify",
  approval: "Awaiting owner",
};

function Widget(props: {
  readonly metrics: FlowMetrics | null;
  readonly note?: string;
  readonly range?: MetricsRange;
}): ReactElement {
  const state: MetricsState = {
    range: props.range ?? "week",
    setRange: () => {},
    metrics: props.metrics,
    loaded: true,
    note: props.note ?? null,
    error: null,
  };
  return (
    <div className="dash" style={{ width: 1000, padding: 16 }}>
      <div className="dash-grid">
        <FlowWidget state={state} columnName={(key) => names[key] ?? key} onBoard={() => {}} />
      </div>
    </div>
  );
}

/** A working week: stats, cycle time by column, work in progress, aging. */
export const Week: Story = () => <Widget metrics={flowMetrics()} />;
/** Four weeks: 28 days of work in progress. */
export const FourWeeks: Story = () => <Widget metrics={flowMetrics4w()} range="4w" />;
/** Nothing has reached Done yet. */
export const Empty: Story = () => <Widget metrics={emptyFlow()} />;
/** Off the board's home, with the home away. */
export const HomeAway: Story = () => (
  <Widget metrics={null} note="Can't reach mac right now, so these numbers can't be shown." />
);
