import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ReactElement } from "react";
import { describe, expect, it, vi } from "vitest";
import type { FlowMetrics, MetricsRange } from "../../protocol/metrics";
import { emptyFlow, flowMetrics, flowMetrics4w } from "../../test/flowFixtures";
import { FakeDaemon } from "../../test/fakeDaemon";
import FlowWidget, { duration, spelled } from "./FlowWidget";
import { useMetrics } from "./useMetrics";

function Harness(props: {
  readonly client: FakeDaemon;
  readonly onBoard: () => void;
}): ReactElement {
  const state = useMetrics(props.client, "p1", true);
  return (
    <FlowWidget state={state} columnName={(key) => key.toUpperCase()} onBoard={props.onBoard} />
  );
}

function setup(
  reply: (range: MetricsRange) => { metrics: FlowMetrics | null; note: string | null },
) {
  const client = new FakeDaemon().onRequest("metrics_get", (body) => {
    const range = (body as { readonly range: MetricsRange }).range;
    return { type: "metrics", req_id: "1", ...reply(range) };
  });
  const onBoard = vi.fn<() => void>();
  render(<Harness client={client} onBoard={onBoard} />);
  return { client, onBoard };
}

const flow = (): HTMLElement => screen.getByRole("region", { name: /^Flow/ });

describe("FlowWidget", () => {
  it("shows items done, cycle time in plain words, rework and expired tasks", async () => {
    setup(() => ({ metrics: flowMetrics(), note: null }));
    expect(await screen.findByText("85% done within 3.2 days")).toBeInTheDocument();
    expect(flow()).toHaveTextContent("Cycle time1.6 days typical");
    expect(flow()).toHaveTextContent("Throughput7 items done");
    expect(flow()).toHaveTextContent("Last 8 weeks");
    expect(flow()).toHaveTextContent("Rework13%1 of 8 sent back");
    expect(flow()).toHaveTextContent("Expired tasks2tasks that ran out of time");
    expect(flow()).not.toHaveTextContent("p50");
    const weeks = within(flow()).getByRole("list", { name: "Done per week, last 8 weeks" });
    expect(within(weeks).getAllByRole("listitem").at(-1)).toHaveAttribute(
      "title",
      "This week: 7 done",
    );
  });

  it("charts time in each column, work in progress each day with its scale, and aging", async () => {
    const user = userEvent.setup();
    const { onBoard } = setup(() => ({ metrics: flowMetrics(), note: null }));
    const columns = await screen.findByRole("list", { name: "Time in each column" });
    const doing = within(columns).getAllByRole("listitem")[0];
    expect(doing).toHaveTextContent("Doing1.2d typical · 85% within 2.4d");
    expect(doing).toHaveAccessibleName("Doing: typical 1.2 days, 85% within 2.4 days, 9 items");
    const wip = screen.getByRole("list", { name: "In progress at the end of each day" });
    expect(within(wip).getAllByRole("listitem")).toHaveLength(7);
    expect(flow()).toHaveTextContent("In progress each day");
    expect(flow()).toHaveTextContent("Now 3 · peak 4 · this week");
    await user.click(screen.getByRole("button", { name: /H-014 Pairing over Tailscale/ }));
    expect(onBoard).toHaveBeenCalled();
    expect(flow()).toHaveTextContent("REVIEW 3.1d");
  });

  it("reads four weeks when asked, and shows the numbers as tables", async () => {
    const user = userEvent.setup();
    const { client } = setup((range) => ({
      metrics: range === "4w" ? flowMetrics4w() : flowMetrics(),
      note: null,
    }));
    await screen.findByText("85% done within 3.2 days");
    await user.click(screen.getByRole("button", { name: "4 weeks" }));
    await waitFor(() => {
      expect(flow()).toHaveTextContent("Throughput20 items done");
    });
    expect(flow()).toHaveTextContent("· last 4 weeks");
    expect(client.requests.at(-1)?.body).toMatchObject({ type: "metrics_get", range: "4w" });
    expect(screen.getByRole("button", { name: "4 weeks" })).toHaveAttribute("aria-pressed", "true");

    await user.click(screen.getByRole("button", { name: "Table view" }));
    const table = screen.getByRole("table", { name: "Time in each column" });
    expect(within(table).getAllByRole("row")).toHaveLength(5);
    expect(within(table).getByRole("columnheader", { name: "Typical (p50)" })).toBeInTheDocument();
    expect(
      within(table).getByRole("columnheader", { name: "85% within (p85)" }),
    ).toBeInTheDocument();
    expect(screen.getByRole("table", { name: "In progress by day" })).toBeInTheDocument();
    expect(screen.queryByRole("list", { name: "Time in each column" })).toBeNull();
  });

  it("says when there's nothing to measure yet, without toggles that do nothing", async () => {
    setup(() => ({ metrics: emptyFlow(), note: null }));
    expect(
      await screen.findByText("Metrics appear after the first item reaches Done."),
    ).toBeInTheDocument();
    expect(within(flow()).queryByRole("button")).toBeNull();
  });

  it("names the home when the numbers live there and it's away", async () => {
    setup(() => ({
      metrics: null,
      note: "Can't reach mac right now, so these numbers can't be shown.",
    }));
    expect(
      await screen.findByText("Can't reach mac right now, so these numbers can't be shown."),
    ).toBeInTheDocument();
    expect(within(flow()).queryByRole("group", { name: "Range" })).toBeNull();
    expect(within(flow()).queryByRole("button", { name: "Table view" })).toBeNull();
  });

  it("writes durations the way people read them", () => {
    expect(duration(1.6 * 86_400)).toBe("1.6d");
    expect(duration(5 * 3600)).toBe("5h");
    expect(duration(40 * 60)).toBe("40m");
    expect(duration(0, 0)).toBe("—");
    expect(spelled(1.6 * 86_400)).toBe("1.6 days");
    expect(spelled(3600)).toBe("1 hour");
    expect(spelled(40 * 60)).toBe("40 minutes");
  });
});
