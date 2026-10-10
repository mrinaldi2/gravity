import { create } from "@bufbuild/protobuf";
import type { MessageInitShape } from "@bufbuild/protobuf";
import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { PrPush } from "../../protocol/gen/hermes/pr/v1/pr_pb";
import { PrPushSchema, PullRequestSchema, Verdict } from "../../protocol/gen/hermes/pr/v1/pr_pb";
import type { Release } from "../../protocol/releases";
import type { FakeDaemon } from "../../test/fakeDaemon";
import * as fx from "../../test/fixtures";
import { prDaemon } from "../../test/prFixtures";
import { MAIN_RECORDS, mainRelease } from "../../test/releaseMainFixtures";
import { actionToastSpy } from "../../test/spies";
import ReleasesView from "./ReleasesView";

function push(init: MessageInitShape<typeof PrPushSchema>["push"]): PrPush {
  return create(PrPushSchema, { push: init });
}

function daemon(records = [...MAIN_RECORDS.values()]): FakeDaemon {
  const releases: readonly Release[] = [mainRelease()];
  return prDaemon([], records).onRequest("list_releases", () => ({
    type: "releases",
    req_id: "1",
    releases,
  }));
}

function renderTab(client: FakeDaemon, onOpenPr?: (n: number) => void): void {
  render(
    <ReleasesView
      client={client}
      project={fx.project({ id: "p1", name: "The Hermes" })}
      bots={[fx.bot({ id: "ops", name: "DevOps", project_id: "p1" })]}
      connected
      canControl
      addToast={actionToastSpy()}
      onOpenPr={onOpenPr}
    />,
  );
}

const row = (n: number): HTMLElement => {
  const li = document.querySelector<HTMLElement>(`[data-pr="${n}"]`);
  if (!li) {
    throw new Error(`no row #${n}`);
  }
  return li;
};

const chips = (n: number): readonly (string | null)[] =>
  within(row(n))
    .queryAllByRole("listitem")
    .map((c) => c.textContent);

describe("a release cut from main, with its PRs' records (H-278)", () => {
  it("reads each PR's record and shows its review chips", async () => {
    const client = daemon();
    renderTab(client);
    expect(
      await screen.findByRole(
        "heading",
        {
          name: "What's in it · 3 pull requests since 5c4b3a2 · all reviewed",
        },
        { timeout: 5000 },
      ),
    ).toBeInTheDocument();
    expect(chips(42)).toEqual(["✓ You", "✓ CE", "✓ UX", "✓ QA"]);
    expect(chips(43)).toEqual(["✓ Architect"]);
    const asked = client.prCalls.map((c) => (c.case === "prGet" ? c.value.number : null));
    expect(new Set(asked)).toEqual(new Set([40, 42, 43]));
    expect(asked).toHaveLength(3);
  });

  it("reads a PR again on its pr_updated, and only that PR", async () => {
    const changed = create(PullRequestSchema, {
      number: 43,
      requiredRoles: ["architect"],
      reviews: [{ role: "architect", verdict: Verdict.APPROVED, stale: true, sha: "c79c8b5" }],
    });
    const client = daemon();
    renderTab(client);
    await screen.findByRole("heading", { name: /all reviewed/u });
    const reads = client.prCalls.length;
    client.onPr("prGet", () => ({ case: "pr", value: changed }));
    act(() =>
      client.emitPrPush(push({ case: "prUpdated", value: { projectId: "p1", number: 43 } })),
    );
    await waitFor(() => expect(chips(43)).toEqual(["⟳ Architect approved an older commit"]));
    expect(screen.getByRole("heading", { name: /1 not reviewed/u })).toBeInTheDocument();
    expect(client.prCalls).toHaveLength(reads + 1);

    act(() => {
      client.emitPrPush(push({ case: "prUpdated", value: { projectId: "p1", number: 99 } }));
      client.emitPrPush(push({ case: "prUpdated", value: { projectId: "p2", number: 43 } }));
    });
    expect(client.prCalls).toHaveLength(reads + 1);
  });

  it("reads no records from a service without pull requests", async () => {
    const client = daemon();
    client.capabilities = client.capabilities.filter((c) => c !== "pull_requests");
    renderTab(client);
    expect(
      await screen.findByRole("heading", { name: "What's in it · 3 pull requests since 5c4b3a2" }),
    ).toBeInTheDocument();
    expect(chips(42)).toEqual([]);
    expect(client.prCalls).toHaveLength(0);
  });

  it("opens a PR in the Pull requests tab from its number", async () => {
    const onOpenPr = vi.fn<(n: number) => void>();
    renderTab(daemon(), onOpenPr);
    await userEvent.click(
      await screen.findByRole("button", {
        name: "#43 · H-250 Typo in the About box, open pull request",
      }),
    );
    expect(onOpenPr).toHaveBeenCalledWith(43);
  });
});
