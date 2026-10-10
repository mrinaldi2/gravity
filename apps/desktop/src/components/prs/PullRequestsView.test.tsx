import { readFileSync } from "node:fs";
import { create, fromJson } from "@bufbuild/protobuf";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import type { JsonValue, MessageInitShape } from "@bufbuild/protobuf";
import { act, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import {
  PrListSchema,
  PrPushSchema,
  PrState,
  PullRequestSchema,
} from "../../protocol/gen/hermes/pr/v1/pr_pb";
import type { PrPush, PullRequest } from "../../protocol/gen/hermes/pr/v1/pr_pb";
import { CARD_PROJECT } from "../../test/cardFixtures";
import type { FakeDaemon } from "../../test/fakeDaemon";
import {
  HEAD,
  OLDER,
  PR_NOW,
  prDaemon,
  prList,
  readyForYouPr,
  waitingPr,
} from "../../test/prFixtures";
import PullRequestsView from "./PullRequestsView";

function show(client: FakeDaemon, initialNumber?: number) {
  return render(
    <PullRequestsView
      client={client}
      project={CARD_PROJECT}
      connected
      now={PR_NOW}
      initialNumber={initialNumber}
    />,
  );
}

function push(init: MessageInitShape<typeof PrPushSchema>["push"]): PrPush {
  return create(PrPushSchema, { push: init });
}

/** The PR-0 fixtures the daemon's contract tests use (H-265). */
function busFixture(name: string): JsonValue {
  const repo = join(dirname(fileURLToPath(import.meta.url)), "..", "..", "..", "..", "..");
  const path = join(repo, "crates", "bus", "fixtures", "pr", `${name}.json`);
  return JSON.parse(readFileSync(path, "utf8")) as JsonValue;
}

describe("PullRequestsView: the list", () => {
  it("shows each PR with its card, review chips and checks, waiting for you first", async () => {
    show(prDaemon());
    const row = (await screen.findByRole("button", { name: /Open pull request #42/ })).closest(
      "li",
    );
    expect(screen.getByRole("button", { name: "Waiting for you · 1" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    expect(row).not.toBeNull();
    const cells = within(row as HTMLElement);
    expect(cells.getByText("#42")).toBeInTheDocument();
    expect(cells.getByText("H-293")).toBeInTheDocument();
    expect(
      cells.getByText("Desktop Dev · into main · 9 files +412 −58 · docs changed · 3h"),
    ).toBeInTheDocument();
    for (const chip of ["▲ You", "✓ Architect", "✓ UX", "✓ CE", "✓ 5 checks"]) {
      expect(cells.getByText(chip)).toBeInTheDocument();
    }
  });

  it("marks PRs behind main or with conflicts, and lists merged ones", async () => {
    const user = userEvent.setup();
    show(prDaemon());
    await user.click(await screen.findByRole("button", { name: "Open · 3" }));
    expect(screen.getByText("⇣ Needs update with main")).toBeInTheDocument();
    expect(
      screen.getByText("⚠ Has conflicts: waiting for Backend Dev to resolve"),
    ).toBeInTheDocument();
    expect(screen.getByText("✕ Architect asked for changes")).toBeInTheDocument();
    expect(screen.getAllByText("◌ Checks running")).toHaveLength(2);
    expect(screen.getByText("○ You after the update")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Merged · 1" }));
    expect(screen.getByText("✓ Merged")).toBeInTheDocument();
    expect(screen.getByText("✓ You")).toBeInTheDocument();
  });

  it("updates live on pr_updated, and ignores another project's pushes", async () => {
    const client = prDaemon();
    show(client);
    await screen.findByRole("button", { name: "Waiting for you · 1" });
    const reads = client.prCalls.length;
    act(() =>
      client.emitPrPush(push({ case: "prUpdated", value: { projectId: "p2", number: 42 } })),
    );
    expect(client.prCalls).toHaveLength(reads);

    const merged = prList();
    for (const p of merged) {
      if (p.number === 42) {
        p.state = PrState.MERGED;
      }
    }
    client.onPr("prList", () => ({ case: "prList", value: create(PrListSchema, { prs: merged }) }));
    act(() =>
      client.emitPrPush(push({ case: "prUpdated", value: { projectId: "p1", number: 42 } })),
    );
    expect(await screen.findByRole("button", { name: "Merged · 2" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Waiting for you · 0" })).toBeInTheDocument();
  });
});

describe("PullRequestsView: one PR", () => {
  it("shows the waiting line, verdicts at their commits, the change note and checks with the computer", async () => {
    show(prDaemon(prList(), [waitingPr()]), 42);
    expect(await screen.findByRole("status")).toHaveTextContent(
      "Merges to main when every review is in and checks pass. Waiting for: you, and CE.",
    );
    const reviews = within(screen.getByRole("region", { name: "Reviews" }));
    expect(reviews.getAllByText(/Approved/).map((n) => n.textContent)).toEqual([
      "✓ Approved f0678fc",
      "✓ Approved f0678fc",
      "⟳ Approved c79c8b5, an older commit",
    ]);
    expect(reviews.getByText("○ Waiting")).toBeInTheDocument();
    expect(screen.getByRole("region", { name: "Change note" })).toHaveTextContent(
      "Tested: 1223 Rust, 1076 desktop, VR 136/136.",
    );
    const checks = within(screen.getByRole("region", { name: "Checks" }));
    expect(checks.getByText("Windows build")).toBeInTheDocument();
    expect(checks.getByText("win-pc · 9m")).toBeInTheDocument();
    expect(screen.getByRole("region", { name: "Findings" })).toHaveTextContent(
      "Findings · 1 of 2 resolved",
    );
    expect(screen.queryByRole("button", { name: /Approve|Review…/ })).toBeNull();
  });

  it("shows files with +/−, the unified diff, and only what changed since an older approval", async () => {
    const user = userEvent.setup();
    const client = prDaemon(prList(), [waitingPr()]);
    show(client, 42);
    await user.click(await screen.findByRole("tab", { name: "Files · 9" }));
    const files = within(await screen.findByRole("list", { name: "Files" }));
    expect(files.getByText("+2")).toBeInTheDocument();
    expect(files.getByText("binary")).toBeInTheDocument();
    expect(screen.getByRole("region", { name: /Diff of .*ReleaseReview.tsx/ })).toHaveTextContent(
      "const approve",
    );
    await user.selectOptions(screen.getByRole("combobox"), `Since c79c8b5, approved by CE`);
    expect(await screen.findAllByText(/c79c8b5 → f0678fc/)).toHaveLength(2);
    expect(client.prCalls.at(-1)).toMatchObject({
      case: "prDiff",
      value: { number: 42, fromSha: OLDER, toSha: HEAD },
    });
    const after = within(screen.getByRole("list", { name: "Files" }));
    await user.click(after.getByRole("button", { name: /docs\/user\/releases.md/ }));
    expect(screen.getAllByRole("region", { name: /Diff of/ })).toHaveLength(1);
  });

  it("folds checks on earlier commits", async () => {
    const user = userEvent.setup();
    show(prDaemon(prList(), [waitingPr()]), 42);
    await user.click(await screen.findByRole("tab", { name: "Checks · 5" }));
    expect(screen.getByRole("region", { name: "Earlier commits" })).toHaveTextContent(
      "c79c8b5 ✕ 1 check failed",
    );
  });

  it("updates live on check_updated for its head, then shows it waiting for you", async () => {
    const client = prDaemon(prList(), [waitingPr()]);
    show(client, 42);
    expect(await screen.findByRole("status")).toHaveTextContent("Waiting for: you, and CE.");
    client.onPr("prGet", () => ({ case: "pr", value: readyForYouPr() }));
    act(() =>
      client.emitPrPush(
        push({ case: "checkUpdated", value: { projectId: "p1", sha: OLDER, name: "x" } }),
      ),
    );
    act(() =>
      client.emitPrPush(
        push({ case: "checkUpdated", value: { projectId: "p1", sha: HEAD, name: "x" } }),
      ),
    );
    expect(
      await screen.findByText("▲ Waiting for your review", { selector: ".pr-chip" }),
    ).toBeInTheDocument();
    expect(screen.getByRole("status")).toHaveTextContent("Waiting for: you.");
  });

  it("goes back to the list from the crumb", async () => {
    const user = userEvent.setup();
    show(prDaemon(), 42);
    await user.click(await screen.findByRole("button", { name: "The Hermes › Pull requests ›" }));
    expect(await screen.findByRole("list", { name: "Pull requests" })).toBeInTheDocument();
  });
});

describe("PullRequestsView on the PR-0 fixtures (H-265)", () => {
  const decode = (name: string): PullRequest =>
    fromJson(PullRequestSchema, busFixture(name), { ignoreUnknownFields: true });

  it("lists every state the daemon sends", async () => {
    const user = userEvent.setup();
    const list = fromJson(PrListSchema, busFixture("pr_list"), { ignoreUnknownFields: true });
    show(prDaemon(list.prs));
    await user.click(await screen.findByRole("button", { name: "Open · 3" }));
    expect(
      screen.getByText("⚠ Has conflicts: waiting for Desktop Dev to resolve"),
    ).toBeInTheDocument();
    expect(screen.getByText("◌ Merging")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Closed · 1" }));
    expect(screen.getByText("Closed")).toBeInTheDocument();
  });

  it("shows a merged PR's cleanup, held on one computer", async () => {
    show(prDaemon([decode("pr_merged")]), 40);
    expect(await screen.findByText(/^✓ Merged into main · 8 Oct, \d\d:\d\d$/)).toBeInTheDocument();
    expect(screen.getByRole("region", { name: "Cleanup" })).toHaveTextContent(
      "⏸ Cleanup held on imac: Uncommitted changes (salvaged)",
    );
  });

  it("names the conflicting files and the other blockers of an open PR", async () => {
    show(prDaemon([decode("pr_behind")]), 45);
    const banner = await screen.findByRole("status");
    expect(banner).toHaveTextContent("Needs update with main");
    expect(banner).toHaveTextContent("apps/desktop/src/styles/base.css, crates/hermesd/src/app.rs");
    expect(screen.getByText(/Not needed: No screens change \(Team Lead\)/)).toBeInTheDocument();
    expect(screen.getByText(/new commits since the last reported push/)).toBeInTheDocument();
  });
});
