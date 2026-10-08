import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { create } from "@bufbuild/protobuf";
import type { OwnerBlocker, Release } from "../../protocol/releases";
import { ReleaseBriefSchema } from "../../protocol/gen/hermes/home/v1/home_pb";
import { FakeDaemon } from "../../test/fakeDaemon";
import * as fx from "../../test/fixtures";
import { deployment, release } from "../../test/releaseFixtures";
import { actionToastSpy } from "../../test/spies";
import { releasePill } from "../home/homeText";
import { focusReview } from "./ReleaseReview";
import ReleasesView from "./ReleasesView";
import WaitingForYou, { NowLine } from "./WaitingForYou";
import type { WaitingActions } from "./WaitingForYou";
import { nowLine, shortAge } from "./waitingText";

const NOW = Date.parse("2026-10-08T12:00:00Z");
const BOTS: Record<string, string> = { ops: "DevOps", dd: "Desktop Dev" };
const botName = (id: string): string | undefined => BOTS[id];

function blocker(over: Partial<OwnerBlocker>): OwnerBlocker {
  return {
    kind: "run",
    id: "a1",
    title: "Run the full test",
    item_id: "H-244",
    bot: "ops",
    computer: "mac",
    created_at: "2026-10-08T11:35:00Z",
    ...over,
  };
}

function actions(): WaitingActions & {
  onDecision: ReturnType<typeof vi.fn>;
  onNeedsYou: ReturnType<typeof vi.fn>;
} {
  return {
    client: new FakeDaemon(),
    connected: true,
    bots: [],
    addToast: actionToastSpy(),
    onDecision: vi.fn<(id: string) => void>(),
    onNeedsYou: vi.fn<() => void>(),
  };
}

function box(blockers: readonly OwnerBlocker[], a = actions(), onReview = vi.fn<() => void>()) {
  render(
    <WaitingForYou
      release={release({ owner_blockers: blockers })}
      botName={botName}
      actions={a}
      onReview={onReview}
      now={() => NOW}
    />,
  );
  return { a, onReview };
}

describe("Waiting for you (UX-048)", () => {
  it("lists each kind in its words, with who, where and how long", () => {
    box([
      blocker({ kind: "ruling", id: "d0", title: "0.17.5", bot: null, computer: null }),
      blocker({}),
      blocker({
        kind: "decision",
        id: "d1",
        title: "Ship the banner?",
        item_id: "H-241",
        bot: "dd",
        computer: null,
        created_at: "2026-10-08T10:00:00Z",
      }),
    ]);
    const section = screen.getByRole("region", { name: "Waiting for you" });
    expect(within(section).getByRole("heading").textContent).toBe("▲ Waiting for you · 3");
    const rows = within(section).getAllByRole("listitem");
    expect(rows[0]?.textContent).toContain("Test 0.17.5 and rule on it");
    // UX-049 §1: the Run card's title, then where and how long; no reason.
    expect(rows[1]?.textContent).toContain("DevOps asks you to run a command on mac");
    expect(rows[1]?.textContent).toContain("on H-244 · 25m");
    expect(rows[1]?.textContent).not.toContain("Run the full test");
    expect(rows[2]?.textContent).toContain("Decide: Ship the banner?");
    expect(rows[2]?.textContent).toContain("Desktop Dev · on H-241 · 2h");
  });

  it("sends each button where the owner clears it", async () => {
    const user = userEvent.setup();
    const { a, onReview } = box([
      blocker({ kind: "ruling", id: "d0", title: "0.17.5", bot: null }),
      blocker({ kind: "decision", id: "d1", title: "Ship it?" }),
      blocker({ kind: "permission", id: "p1", title: "Bash", bot: "dd" }),
    ]);
    await user.click(screen.getByRole("button", { name: "Review…" }));
    expect(onReview).toHaveBeenCalled();
    await user.click(screen.getByRole("button", { name: "Answer" }));
    expect(a.onDecision).toHaveBeenCalledWith("d1");
    await user.click(screen.getByRole("button", { name: "Review" }));
    expect(a.onNeedsYou).toHaveBeenCalled();
    expect(screen.getByText("Desktop Dev wants to run Bash")).toBeTruthy();
  });

  it("shows three rows, then +n more", async () => {
    const user = userEvent.setup();
    box([1, 2, 3, 4, 5].map((n) => blocker({ id: `a${n}` })));
    expect(screen.getAllByRole("listitem")).toHaveLength(3);
    await user.click(screen.getByRole("button", { name: "+2 more" }));
    expect(screen.getAllByRole("listitem")).toHaveLength(5);
  });

  it("shows nothing when nothing waits (§4)", () => {
    box([]);
    expect(screen.queryByRole("region", { name: "Waiting for you" })).toBeNull();
  });
});

describe("the Now: line (UX-048 §3)", () => {
  const name = botName;
  it("names the owner's part first, in UX-049's words", () => {
    const line = (b: OwnerBlocker): string | null =>
      nowLine(release({ status: "assembling", owner_blockers: [b] }), name);
    expect(line(blocker({}))).toBe("Now: waiting for you to run a command on mac (H-244).");
    expect(line(blocker({ kind: "decision", title: "Ship it?" }))).toBe(
      "Now: waiting for you to decide “Ship it?”.",
    );
    expect(line(blocker({ kind: "question", bot: "dd", item_id: "H-241" }))).toBe(
      "Now: waiting for you to answer Desktop Dev (H-241).",
    );
    expect(line(blocker({ kind: "permission", bot: "dd", title: "Bash" }))).toBe(
      "Now: waiting for you to review Desktop Dev's request to run Bash.",
    );
    expect(line(blocker({ kind: "ruling", title: "0.17.5", bot: null }))).toBe(
      "Now: waiting for you to test 0.17.5 and rule on it.",
    );
  });

  it("names who else it waits on", () => {
    const plan = [
      {
        item_id: "H-241",
        title: "",
        column_key: "doing",
        column_name: "Doing",
        category: "doing",
        assignee: null,
        blocked: false,
        ac_checked: 0,
        ac_total: 0,
        ready: false,
      },
      {
        item_id: "H-243",
        title: "",
        column_key: "review",
        column_name: "Review",
        category: "review",
        assignee: null,
        blocked: false,
        ac_checked: 0,
        ac_total: 0,
        ready: false,
      },
      {
        item_id: "H-245",
        title: "",
        column_key: "doing",
        column_name: "Doing",
        category: "doing",
        assignee: null,
        blocked: false,
        ac_checked: 0,
        ac_total: 0,
        ready: false,
      },
    ];
    expect(
      nowLine(release({ status: "planned", owner_blockers: [], plan, builds: [] }), name),
    ).toBe("Now: 3 items still in progress (H-241 in Doing, H-243 in Review, +1).");
    expect(
      nowLine(release({ status: "assembling", owner_blockers: [], plan: [], builds: [] }), name),
    ).toBe("Now: DevOps is building the packages.");
    const readiness = {
      items_total: 1,
      items_ready: 1,
      builds: ["desktop-mac"],
      tests_required: ["mac", "win-pc"],
      tests_passed: ["mac"],
    };
    expect(nowLine(release({ status: "built", owner_blockers: [], readiness }), name)).toBe(
      "Now: testing on win-pc (1 of 2 computers).",
    );
    const rolling = release({
      status: "deploying",
      owner_blockers: [],
      deploys_to: ["mac", "imac", "win-pc"],
      deployments: [
        deployment({ machine: "mac", result: "ok" }),
        deployment({ machine: "imac", result: "ok" }),
      ],
    });
    expect(nowLine(rolling, name)).toBe("Now: rolling out, 2 of 3 computers updated.");
    // UX-050 §2: never "0 of 0", and never "nothing is blocking" mid-way.
    const plain = (over: Partial<Release>): string | null =>
      nowLine(release({ owner_blockers: [], deploys_to: [], deployments: [], ...over }), name);
    expect(plain({ status: "approved" })).toBe("Now: approved, the rollout starts soon.");
    expect(plain({ status: "deploying" })).toBe("Now: rolling out.");
    const builtUp = { plan: [], builds: release().builds, readiness: undefined };
    expect(plain({ status: "assembling", ...builtUp })).toBe(
      "Now: DevOps is assembling the package.",
    );
    expect(plain({ status: "built", ...builtUp })).toBe("Now: DevOps is assembling the package.");
  });

  it("Review… takes the owner to Approve, or the heading when Approve can't be pressed (UX-050 §1)", () => {
    document.body.innerHTML = `
      <div id="r">
        <header class="release-head"><h2 tabindex="-1">0.17.5</h2></header>
        <footer class="release-bar">
          <button class="btn btn-danger">Reject…</button>
          <button class="btn btn-primary" disabled>Approve 0.17.5</button>
        </footer>
      </div>`;
    const root = document.getElementById("r");
    Element.prototype.scrollIntoView = vi.fn<() => void>();
    focusReview(root);
    expect(document.activeElement?.textContent).toBe("0.17.5");
    root?.querySelector(".btn-primary")?.removeAttribute("disabled");
    focusReview(root);
    expect(document.activeElement?.textContent).toBe("Approve 0.17.5");
  });

  it("points an older service's owner to Needs you", async () => {
    const user = userEvent.setup();
    const onNeedsYou = vi.fn<() => void>();
    const old = release({ status: "assembling" }) as Release;
    render(
      <NowLine
        release={{ ...old, owner_blockers: undefined }}
        botName={name}
        onNeedsYou={onNeedsYou}
      />,
    );
    await user.click(screen.getByRole("button", { name: "Needs you" }));
    expect(onNeedsYou).toHaveBeenCalled();
  });

  it("says how long in short", () => {
    expect(shortAge("2026-10-08T11:35:00Z", NOW)).toBe("25m");
    expect(shortAge("2026-10-08T09:00:00Z", NOW)).toBe("3h");
    expect(shortAge("2026-10-06T09:00:00Z", NOW)).toBe("2d");
  });
});

describe("where else it shows", () => {
  it("the Projects card's pill says the release waits for you", () => {
    const brief = create(ReleaseBriefSchema, {
      version: "0.17.5",
      state: "assembling",
      ownerBlockerCount: 2,
    });
    expect(releasePill(brief).text).toBe("◐ 0.17.5 waits for you");
    expect(releasePill({ ...brief, ownerBlockerCount: 0 }).text).not.toContain("waits for you");
  });

  it("the Releases list marks it, and a release_updated push reads it again", async () => {
    let blockers: readonly OwnerBlocker[] = [blocker({})];
    const client = new FakeDaemon().onRequest("list_releases", () => ({
      type: "releases",
      req_id: "1",
      releases: [
        release({ id: "r1", project_id: "p1", status: "assembling", owner_blockers: blockers }),
      ],
    }));
    client.onBoard("boardGet", () => {
      throw new Error("no board");
    });
    render(
      <ReleasesView
        client={client}
        project={fx.project({ id: "p1" })}
        bots={[]}
        connected
        canControl
        addToast={actionToastSpy()}
      />,
    );
    const nav = await screen.findByRole("navigation", { name: "Releases" });
    await waitFor(() => expect(nav.textContent).toContain("▲ Waiting for you · 1"));
    blockers = [];
    act(() =>
      client.emit("release_updated", {
        type: "release_updated",
        project_id: "p1",
        release_id: "r1",
      }),
    );
    await waitFor(() => expect(nav.textContent).not.toContain("Waiting for you"));
  });
});
