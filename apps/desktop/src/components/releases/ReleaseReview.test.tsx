import { act, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import type { ReactElement } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AddToast } from "../../app/useToasts";
import type { Release, ReleaseRequestBody } from "../../protocol/releases";
import { FakeDaemon } from "../../test/fakeDaemon";
import {
  CANCELLED,
  MAC_SHA,
  RELEASE_TITLES,
  WIN_SHA,
  deployment,
  release,
} from "../../test/releaseFixtures";
import { actionToastSpy } from "../../test/spies";
import { fmtTimestamp } from "../../util";
import ReleaseReview, { sourceLine, targetsLine } from "./ReleaseReview";
import { UNDO_MS } from "./useDelayedSend";

import { useReleaseActions } from "./useReleases";

const NOW = Date.parse("2026-10-05T12:00:00Z");
const BOTS = new Map([
  ["ops", "DevOps"],
  ["tester", "Tester"],
]);

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
      botName={(id) => BOTS.get(id)}
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

/** An item's row in the Items tab. */
function row(id: string): HTMLElement {
  return screen.getByText(id).closest("li") as HTMLElement;
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
    await user.click(within(dialog).getByRole("button", { name: "Approve 0.16.0" }));

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
    const dialog = screen.getByRole("dialog", { name: "Approve 0.16.0?" });
    await user.click(within(dialog).getByRole("button", { name: "Approve 0.16.0" }));
    expect(screen.getByRole("status")).toHaveTextContent("Approving 0.16.0…");
    const undo = addToast.mock.calls[0]?.[3]?.action;
    act(() => undo?.run());
    await act(async () => vi.advanceTimersByTimeAsync(UNDO_MS * 2));
    expect(fake.requests).toHaveLength(0);
  });

  it("approves part of a package with the rest left out", async () => {
    const { fake, user } = setup(release());
    await user.click(within(row("H-020")).getByRole("button", { name: "Leave out" }));
    const dialog = screen.getByRole("dialog", { name: "Leave out H-020?" });
    await user.click(within(dialog).getByLabelText(/Rework/));
    const leave = within(dialog).getByRole("button", { name: "Leave out" });
    expect(leave).toBeDisabled();
    await user.type(within(dialog).getByLabelText("What needs rework"), "font falls back");
    await user.click(leave);

    await user.click(screen.getByRole("button", { name: "Approve 1 of 2 items" }));
    const approve = screen.getByRole("dialog", { name: "Approve 1 of 2 items?" });
    expect(approve).toHaveTextContent("DevOps repackages the 1 approved item without the rest");
    await user.click(within(approve).getByRole("button", { name: "Approve 1 item" }));
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
        { machine: "mac", tester: "tester", build_sha256: MAC_SHA, result: "pass" },
        { machine: "win-pc", tester: "tester-win", build_sha256: WIN_SHA, result: "fail" },
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
    const note = screen.getByText(
      "You can approve, hold or reject this package only from a device connected directly to Mac.",
    );
    for (const name of ["Approve 0.16.0", "Hold", "Reject…"]) {
      expect(screen.getByRole("button", { name })).toHaveAttribute("aria-describedby", note.id);
    }
    expect(screen.queryByRole("button", { name: "Leave out" })).not.toBeInTheDocument();
  });

  it("says when this device has no approve access", () => {
    setup(release({ can_rule: false, rule_on: null }));
    expect(
      screen.getByText("This device can't rule on releases: it doesn't have approve access."),
    ).toBeInTheDocument();
  });

  it("names each computer's own build, and a tester it doesn't know as unknown", () => {
    setup(release());
    const tests = screen.getByRole("region", { name: "Tests" });
    expect(within(tests).getByText("Tester · desktop-mac 0.16.0")).toBeInTheDocument();
    expect(within(tests).getByText("Unknown tester · desktop-win 0.16.0")).toBeInTheDocument();
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
    const remind = "2026-10-20T09:00:00Z";
    const held = release({ status: "held", held_note: "after the trip", remind_at: remind });
    const { fake, user } = setup(held, release());
    expect(screen.getAllByRole("status")[0]).toHaveTextContent(
      `⏸ On hold: “after the trip”. Reminds you ${fmtTimestamp(remind)}.`,
    );
    await user.click(screen.getByRole("button", { name: "Take off hold" }));
    expect(sent(fake)).toEqual([{ type: "release_unhold", release_id: "rel-1" }]);
  });

  it("says a held package is on hold, without a note or a reminder", () => {
    setup(release({ status: "held" }));
    expect(screen.getAllByRole("status")[0]).toHaveTextContent(/^⏸ On hold\.$/);
  });

  it("shows a paused rollout on every target computer and resumes it", async () => {
    const paused = release({
      status: "paused",
      paused_reason: "crash on launch on win-pc",
      deployments: [deployment({ result: "ok" })],
    });
    const { fake, user } = setup(paused, release({ status: "deploying" }));
    expect(screen.getAllByRole("status")[0]).toHaveTextContent(
      /^⏸ Rollout paused: crash on launch on win-pc\.Resume rollout$/,
    );
    const rollout = screen.getByRole("tabpanel");
    expect(rollout).toHaveTextContent("mac✓ Live");
    expect(rollout).toHaveTextContent("win-pc○ Not started");
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
    expect(screen.getAllByRole("status")[0]).toHaveTextContent(
      "You approved part of this package. DevOps will build a new one without H-020, and you'll rule on that build.",
    );
    expect(
      within(screen.getByRole("list", { name: "What happened" })).getByText(
        "DevOps cancelled 0.16.1, the package that was going to replace this one. Their note: “took H-021 along”.",
      ),
    ).toBeInTheDocument();
  });

  it("lists what is approved unproven, and a lead's own-evidence tick", () => {
    setup(
      release({
        status: "deploying",
        post_install: [
          {
            item_id: "H-017",
            index: 1,
            text: "survives a reboot",
            checked: false,
            checked_by: null,
          },
          {
            item_id: "H-020",
            index: 0,
            text: "installs per machine",
            checked: true,
            checked_by: "bot:tester",
          },
        ],
        events: [
          {
            release_id: "rel-1",
            release_name: "0.16.0",
            related_id: null,
            kind: "lead_ticked",
            actor: "ops",
            note: "CI run 812",
            detail: { item_id: "H-017", text: "survives a reboot", passed: true },
            at: "2026-10-05T11:00:00Z",
          },
        ],
      }),
    );
    const unproven = screen.getByRole("region", { name: "Checked after install" });
    expect(unproven).toHaveTextContent("so you approve these unproven");
    expect(unproven).toHaveTextContent("Not checked yet: H-017 survives a reboot");
    expect(unproven).toHaveTextContent("Checked: H-020 installs per machine · Tester");
    expect(screen.getByRole("list", { name: "What happened" })).toHaveTextContent(
      "DevOps ticked “survives a reboot” on H-017 on the lead's own evidence. Evidence: “CI run 812”.",
    );
  });

  it("words each item's ruling as the choice was made: included or left out", async () => {
    const { user } = setup(
      release({
        status: "repackaging",
        items: [
          { item_id: "H-017", verdict: "ship", owner_note: null },
          { item_id: "H-020", verdict: "rework", owner_note: "font falls back on win-pc" },
          { item_id: "H-021", verdict: "hold", owner_note: null },
        ],
      }),
    );
    await user.click(screen.getByRole("tab", { name: "Items 3" }));
    expect(row("H-017")).toHaveTextContent("✓ Included");
    expect(row("H-020")).toHaveTextContent(
      "⤼ Left out · back to Doing: “font falls back on win-pc”",
    );
    expect(row("H-021")).toHaveTextContent("⤼ Left out · waits for the next package");
  });

  it("lists computers the rollout hasn't reached as queued while it rolls out", () => {
    setup(release({ status: "deploying", deployments: [deployment({ result: "ok" })] }));
    expect(screen.getByRole("tabpanel")).toHaveTextContent("win-pc○ Queued");
  });
});

describe("the commit a release was built from", () => {
  it("names it with the release branch, or says why it can't land", () => {
    const r = release();
    expect(sourceLine(r, "0.16.0")).toBe("Built from 1a2b3c4 on release/desktop-0.16.0");
    const [mac, win] = r.builds;
    if (mac === undefined || win === undefined) {
      throw new Error("the fixture has two builds");
    }
    const unnamed = release({ builds: [{ ...mac, source_commit: null }, win] });
    expect(sourceLine(unnamed, "0.16.0")).toMatch(/not recorded for every build/u);
    const split = release({ builds: [{ ...mac, source_commit: "f".repeat(40) }, win] });
    expect(sourceLine(split, "0.16.0")).toMatch(/more than one commit/u);
  });
});

describe("the computers frozen into a package", () => {
  it("says where it was tested, who chose that, and where it goes", () => {
    expect(targetsLine(release())).toBeNull();
    const narrowed = release({
      tested_on: ["imac"],
      tested_set_by: "lead",
      deploys_to: ["imac", "mac"],
      deploys_set_by: null,
    });
    expect(targetsLine(narrowed)).toBe("Tested on imac (chosen by the lead) · Goes to imac, mac");
    setup(narrowed);
    expect(screen.getByText(/Tested on imac/u)).toBeInTheDocument();
  });
});
