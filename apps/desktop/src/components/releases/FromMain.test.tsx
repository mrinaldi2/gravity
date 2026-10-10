import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import type { ReactElement } from "react";
import { describe, expect, it, vi } from "vitest";
import type { Release } from "../../protocol/releases";
import { FakeDaemon } from "../../test/fakeDaemon";
import { MAIN_RECORDS, MAIN_TITLES, mainRelease } from "../../test/releaseMainFixtures";
import type { PrRecords } from "./releaseMain";
import ReleaseReview from "./ReleaseReview";
import { useReleaseActions } from "./useReleases";

function Harness(props: {
  readonly initial: Release;
  readonly client: FakeDaemon;
  readonly records?: PrRecords;
  readonly onOpenPr?: (n: number) => void;
}): ReactElement {
  const [current, setCurrent] = useState(props.initial);
  const actions = useReleaseActions(props.client, () => undefined, setCurrent);
  return (
    <ReleaseReview
      release={current}
      titles={MAIN_TITLES}
      botName={(id) => (id === "ops" ? "DevOps" : undefined)}
      actions={actions}
      canControl
      previous="0.17.5"
      records={props.records}
      onOpenPr={props.onOpenPr}
    />
  );
}

const section = (): HTMLElement => screen.getByRole("region", { name: "What's in it" });
const row = (n: number): HTMLElement => {
  const li = section().querySelector<HTMLElement>(`[data-pr="${n}"]`);
  if (!li) {
    throw new Error(`no row #${n}`);
  }
  return li;
};

describe("a release cut from main (H-278)", () => {
  it("names its tag and commit, and lists what's in it, also included and not merged", () => {
    render(<Harness initial={mainRelease()} client={new FakeDaemon()} records={MAIN_RECORDS} />);
    expect(
      screen.getByText("tag desktop-v0.18.0 on main at 9a1b2c3 · DevOps tags it once you approve"),
    ).toBeInTheDocument();
    expect(
      within(section()).getByRole("heading", {
        name: "What's in it · 3 pull requests since 0.17.5 · all reviewed",
      }),
    ).toBeInTheDocument();
    expect(row(42)).toHaveTextContent("H-247 Waiting for you on releases");
    expect(row(42)).toHaveTextContent("merged · 8f35e95");
    expect(row(42)).toHaveTextContent("✓ Included");
    expect(
      within(row(42))
        .getAllByRole("listitem")
        .map((c) => c.textContent),
    ).toEqual(["✓ You", "✓ CE", "✓ UX", "✓ QA"]);
    expect(
      screen.getByRole("heading", {
        name: "Also included · 1 merged pull request not planned for this release",
      }),
    ).toBeInTheDocument();
    expect(row(43)).toHaveTextContent("H-250 Typo in the About box");
    expect(screen.getByRole("heading", { name: "Not merged yet · 1 planned card" })).toBeVisible();
    expect(section()).toHaveTextContent("H-259 Docs tab");
    // Leave out is per pull request now; the items keep no Leave out of their own.
    expect(screen.queryByRole("button", { name: "Leave out" })).not.toBeInTheDocument();
  });

  it("shows no chips and no review clause without the PRs' records", () => {
    render(<Harness initial={mainRelease()} client={new FakeDaemon()} />);
    expect(
      within(section()).getByRole("heading", {
        name: "What's in it · 3 pull requests since 0.17.5",
      }),
    ).toBeInTheDocument();
    expect(within(row(42)).queryByRole("list")).not.toBeInTheDocument();
    expect(within(row(42)).queryByRole("button", { name: /open pull request/u })).toBeNull();
  });

  it("opens a PR from its number, named by its visible text first", async () => {
    const user = userEvent.setup();
    const onOpenPr = vi.fn<(n: number) => void>();
    render(<Harness initial={mainRelease()} client={new FakeDaemon()} onOpenPr={onOpenPr} />);
    await user.click(
      screen.getByRole("button", {
        name: "#42 · H-247 Waiting for you on releases, open pull request",
      }),
    );
    expect(onOpenPr).toHaveBeenCalledWith(42);
  });

  it("leaves a PR out over the app's connection, after saying what happens", async () => {
    const user = userEvent.setup();
    const initial = mainRelease({ also_included: [] });
    const after = mainRelease({ status: "assembling", also_included: [] });
    const client = new FakeDaemon()
      .onRequest("release_leave_out", () => ({
        type: "leave_out",
        req_id: "1",
        result: { mode: "revert" },
      }))
      .onRequest("get_release", () => ({ type: "release", req_id: "2", release: after }));
    render(<Harness initial={initial} client={client} />);
    const leave = within(row(40)).getByRole("button", { name: "Leave out #40…" });
    await user.click(leave);
    const dialog = screen.getByRole("dialog", {
      name: "Leave out #40 · H-203 Card ids as links?",
    });
    expect(dialog).toHaveTextContent(
      "DevOps opens an undo pull request, which goes through the merge queue",
    );
    // Cancel has focus, so a stray Enter never leaves anything out.
    expect(within(dialog).getByRole("button", { name: "Cancel" })).toHaveFocus();
    await user.click(within(dialog).getByRole("button", { name: "Leave out #40" }));
    await waitFor(() => expect(client.requests).toHaveLength(2));
    expect(client.requests[0]?.body).toEqual({
      type: "release_leave_out",
      project_id: "p1",
      release_id: "rel-18",
      prs: [40],
    });
    expect(client.requests[1]?.body).toEqual({ type: "get_release", release_id: "rel-18" });
    expect(within(row(40)).getByRole("button", { name: "Leave out #40…" })).toHaveFocus();
  });

  it("returns focus to the row's Leave out when the confirmation is cancelled", async () => {
    const user = userEvent.setup();
    const client = new FakeDaemon();
    render(<Harness initial={mainRelease()} client={client} />);
    await user.click(within(row(43)).getByRole("button", { name: "Leave out #43…" }));
    expect(screen.getByRole("dialog")).toHaveTextContent(
      "If #43 merged after everything 0.18.0 keeps, 0.18.0 is cut again just before it.",
    );
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(within(row(43)).getByRole("button", { name: "Leave out #43…" })).toHaveFocus();
    expect(client.requests).toHaveLength(0);
  });

  it("offers no Leave out once tagged, or where this device can't rule", () => {
    const { unmount } = render(
      <Harness initial={mainRelease({ tag: "desktop-v0.18.0" })} client={new FakeDaemon()} />,
    );
    expect(screen.queryByRole("button", { name: /^Leave out #/u })).toBeNull();
    unmount();
    render(<Harness initial={mainRelease({ can_rule: false })} client={new FakeDaemon()} />);
    expect(screen.queryByRole("button", { name: /^Leave out #/u })).toBeNull();
  });
});
