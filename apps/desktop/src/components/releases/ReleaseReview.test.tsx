import { act, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import type { ReactElement } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AddToast } from "../../app/useToasts";
import type { Release, ReleaseRequestBody } from "../../protocol/releases";
import { FakeDaemon } from "../../test/fakeDaemon";
import { CANCELLED, RELEASE_TITLES, deployment, release } from "../../test/releaseFixtures";
import { actionToastSpy } from "../../test/spies";
import ReleaseReview from "./ReleaseReview";
import { UNDO_MS } from "./useDelayedSend";
import { useReleaseActions } from "./useReleases";

const NOW = Date.parse("2026-10-05T12:00:00Z");

function Harness(props: {
  readonly initial: Release;
  readonly client: FakeDaemon;
  readonly addToast: AddToast;
  readonly canControl?: boolean;
}): ReactElement {
  const [current, setCurrent] = useState(props.initial);
  const actions = useReleaseActions(props.client, props.addToast, setCurrent);
  return (
    <ReleaseReview
      release={current}
      titles={RELEASE_TITLES}
      botName={(id) => (id === "ops" ? "DevOps" : id)}
      actions={actions}
      canControl={props.canControl ?? true}
      now={() => NOW}
    />
  );
}

/** A daemon that answers every release request with `next`. */
function daemon(next: Release): FakeDaemon {
  const fake = new FakeDaemon();
  for (const type of [
    "release_rule",
    "release_hold",
    "release_unhold",
    "release_pause",
    "release_resume",
  ] as const) {
    fake.onRequest(type, () => ({ type: "release", req_id: "1", release: next }));
  }
  return fake;
}

function sent(fake: FakeDaemon): ReleaseRequestBody[] {
  return fake.requests.map((r) => r.body as ReleaseRequestBody);
}

function setup(initial: Release, next: Release = initial, canControl = true) {
  const fake = daemon(next);
  const addToast = actionToastSpy();
  const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
  render(<Harness initial={initial} client={fake} addToast={addToast} canControl={canControl} />);
  return { fake, addToast, user };
}

describe("ReleaseReview", () => {
  beforeEach(() => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it("approves every item after the Undo window, never before", async () => {
    const { fake, addToast, user } = setup(release(), release({ status: "approved" }));
    expect(screen.getByText("All 2 computers passed.")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Approve 0.16.0" }));
    const dialog = screen.getByRole("dialog", { name: "Approve 0.16.0?" });
    expect(within(dialog).getByRole("button", { name: "Cancel" })).toHaveFocus();
    await user.click(within(dialog).getByRole("button", { name: "Approve" }));

    expect(addToast).toHaveBeenCalledWith("info", "Approving 0.16.0…", expect.any(String), {
      action: expect.objectContaining({ label: "Undo" }),
    });
    expect(fake.requests).toHaveLength(0);
    await act(async () => vi.advanceTimersByTimeAsync(UNDO_MS));
    expect(sent(fake)).toEqual([
      {
        type: "release_rule",
        release_id: "rel-1",
        verdicts: [
          { item_id: "H-017", verdict: "ship" },
          { item_id: "H-020", verdict: "ship" },
        ],
        expected_version: 4,
      },
    ]);
    expect(await screen.findByText(/Approved: rolling out soon/)).toBeInTheDocument();
  });

  it("sends nothing when Undo is pressed", async () => {
    const { fake, addToast, user } = setup(release());
    await user.click(screen.getByRole("button", { name: "Approve 0.16.0" }));
    await user.click(screen.getByRole("button", { name: "Approve" }));
    expect(screen.getByRole("status")).toHaveTextContent("Approving 0.16.0…");
    const undo = addToast.mock.calls[0]?.[3]?.action;
    act(() => undo?.run());
    await act(async () => vi.advanceTimersByTimeAsync(UNDO_MS * 2));
    expect(fake.requests).toHaveLength(0);
  });

  it("approves part of a package with the rest left out", async () => {
    const { fake, user } = setup(release());
    const row = screen.getByText("H-020").closest("li");
    await user.click(within(row as HTMLElement).getByRole("button", { name: "Leave out" }));
    const dialog = screen.getByRole("dialog", { name: "Leave out H-020?" });
    await user.click(within(dialog).getByLabelText(/Rework/));
    const leave = within(dialog).getByRole("button", { name: "Leave out" });
    expect(leave).toBeDisabled();
    await user.type(within(dialog).getByLabelText("What needs rework"), "font falls back");
    await user.click(leave);

    await user.click(screen.getByRole("button", { name: "Approve 1 of 2 items" }));
    await user.click(screen.getByRole("button", { name: "Approve 1 items" }));
    await act(async () => vi.advanceTimersByTimeAsync(UNDO_MS));
    expect(sent(fake)[0]).toMatchObject({
      type: "release_rule",
      verdicts: [
        { item_id: "H-017", verdict: "ship" },
        { item_id: "H-020", verdict: "rework", note: "font falls back" },
      ],
    });
  });

  it("warns about a failed computer before approving", async () => {
    const failing = release({
      tests: [
        { machine: "mac", tester: "tester", build_sha256: "a".repeat(64), result: "pass" },
        { machine: "win-pc", tester: "tester-win", build_sha256: "a".repeat(64), result: "fail" },
      ],
    });
    const { user } = setup(failing);
    expect(screen.getByText(/1 computer didn't pass: win-pc/)).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Approve 0.16.0" }));
    expect(screen.getByText(/win-pc didn't pass. Approve anyway/)).toBeInTheDocument();
  });

  it("opens the approval with ⌘↩ while the review has focus", async () => {
    const { user } = setup(release());
    await user.keyboard("{Meta>}{Enter}{/Meta}");
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    screen.getByRole("tab", { name: "Changelog" }).focus();
    await user.keyboard("{Meta>}{Enter}{/Meta}");
    expect(screen.getByRole("dialog", { name: "Approve 0.16.0?" })).toBeInTheDocument();
  });

  it("disables ruling where this connection can't rule, and says where it can", () => {
    setup(release({ can_rule: false, rule_on: "Mac" }));
    expect(screen.getByRole("button", { name: "Approve 0.16.0" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Hold" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Reject…" })).toBeDisabled();
    expect(screen.getByText("Rule on it from a device connected to Mac.")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Leave out" })).not.toBeInTheDocument();
  });

  it("holds with a reminder, after the Undo window", async () => {
    const { fake, user } = setup(release(), release({ status: "held" }));
    await user.click(screen.getByRole("button", { name: "Hold" }));
    const dialog = screen.getByRole("dialog", { name: "Hold 0.16.0?" });
    await user.type(within(dialog).getByLabelText("Reason (optional)"), "after the trip");
    await user.selectOptions(within(dialog).getByLabelText("Remind me"), "Tomorrow");
    await user.click(within(dialog).getByRole("button", { name: "Hold" }));
    await act(async () => vi.advanceTimersByTimeAsync(UNDO_MS));
    expect(sent(fake)).toEqual([
      {
        type: "release_hold",
        release_id: "rel-1",
        note: "after the trip",
        remind_at: new Date(NOW + 86_400_000).toISOString(),
      },
    ]);
  });

  it("rejects only with a reason, copied to every item", async () => {
    const { fake, user } = setup(release());
    await user.click(screen.getByRole("button", { name: "Reject…" }));
    const dialog = screen.getByRole("dialog", { name: "Reject 0.16.0?" });
    const reject = within(dialog).getByRole("button", { name: "Reject" });
    expect(reject).toBeDisabled();
    await user.type(within(dialog).getByLabelText(/Reason/), "not this week");
    await user.selectOptions(within(dialog).getByLabelText("Where H-020 goes"), "Back to Ready");
    await user.click(reject);
    await act(async () => vi.advanceTimersByTimeAsync(UNDO_MS));
    expect(sent(fake)[0]).toMatchObject({
      verdicts: [
        { item_id: "H-017", verdict: "rework", note: "not this week" },
        { item_id: "H-020", verdict: "hold", note: "not this week" },
      ],
    });
  });

  it("takes a held package off hold at once", async () => {
    const held = release({ status: "held", held_note: "after the trip" });
    const { fake, user } = setup(held, release());
    expect(screen.getByText(/after the trip/)).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Take off hold" }));
    expect(sent(fake)).toEqual([{ type: "release_unhold", release_id: "rel-1" }]);
  });

  it("shows a paused rollout and resumes it", async () => {
    const paused = release({
      status: "paused",
      paused_reason: "crash on launch",
      deployments: [deployment()],
    });
    const { fake, user } = setup(paused, release({ status: "deploying" }));
    expect(screen.getByText(/crash on launch/)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Approve/ })).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Resume rollout" }));
    expect(sent(fake)).toEqual([{ type: "release_resume", release_id: "rel-1" }]);
  });

  it("says what a repackaging package waits for, and which successor was cancelled", () => {
    render(
      <Harness
        initial={release({
          status: "repackaging",
          items: [
            { item_id: "H-017", verdict: "ship", owner_note: null },
            { item_id: "H-020", verdict: "rework", owner_note: "font" },
          ],
          events: [CANCELLED],
        })}
        client={daemon(release())}
        addToast={actionToastSpy()}
      />,
    );
    expect(screen.getByText(/building a new one without H-020/)).toBeInTheDocument();
    expect(
      within(screen.getByRole("list", { name: "What happened" })).getByText(
        "DevOps cancelled 0.16.1, a package that would have replaced this one: took H-021 along",
      ),
    ).toBeInTheDocument();
  });
});
