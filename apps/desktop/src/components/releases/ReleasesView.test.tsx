import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { Decision } from "../../protocol/decisions";
import type { Release } from "../../protocol/releases";
import { card, snapshot } from "../../test/boardFixtures";
import * as dfx from "../../test/decisionFixtures";
import { FakeDaemon } from "../../test/fakeDaemon";
import * as fx from "../../test/fixtures";
import { release } from "../../test/releaseFixtures";
import { actionToastSpy } from "../../test/spies";
import ControlCenterView from "../control/ControlCenterView";
import ReleasesView from "./ReleasesView";

function daemon(releases: readonly Release[]): FakeDaemon {
  return new FakeDaemon()
    .onRequest("list_releases", () => ({ type: "releases", req_id: "1", releases }))
    .onBoard("boardGet", () => ({
      case: "board",
      value: snapshot([card({ id: "H-017", title: "Release packages and the deploy gate" })]),
    }));
}

function renderTab(releases: readonly Release[]) {
  const client = daemon(releases);
  render(
    <ReleasesView
      client={client}
      project={fx.project({ id: "p1", name: "The Hermes" })}
      bots={[fx.bot({ id: "ops", name: "DevOps", project_id: "p1" })]}
      connected
      canControl
      addToast={actionToastSpy()}
    />,
  );
  return client;
}

describe("ReleasesView", () => {
  it("lists current packages before history and reviews the current one", async () => {
    renderTab([
      release(),
      release({ id: "rel-0", display_version: "0.15.2", status: "deployed", decision_id: null }),
    ]);
    const nav = await screen.findByRole("navigation", { name: "Releases" });
    expect(nav).toHaveTextContent(/Current.*0\.16\.0.*History.*0\.15\.2/s);
    expect(screen.getByRole("heading", { name: "0.16.0" })).toBeInTheDocument();
    expect(screen.getByText("Packaged by DevOps", { exact: false })).toBeInTheDocument();
    expect(await screen.findByText("Release packages and the deploy gate")).toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: /0\.15\.2/ }));
    expect(screen.getByRole("heading", { name: "0.15.2" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Approve/ })).not.toBeInTheDocument();
  });

  it("says where packages come from when there are none", async () => {
    const client = renderTab([]);
    expect(await screen.findByRole("heading", { name: "No release yet" })).toBeInTheDocument();
    // No package, so nothing asks the board for item titles.
    expect(client.boardCalls).toHaveLength(0);
  });
});

function decisions(list: readonly Decision[], releases: readonly Release[]): FakeDaemon {
  return daemon(releases)
    .onRequest("list_decisions", () => ({ type: "decisions", req_id: "1", decisions: list }))
    .onRequest("get_decision", (body) => ({
      type: "decision",
      req_id: "1",
      decision:
        list.find((d) => body.type === "get_decision" && d.id === body.decision_id) ??
        dfx.decision(),
    }))
    .onRequest("list_tags", () => ({ type: "tags", req_id: "1", tags: [] }));
}

function renderDecisions(client: FakeDaemon, decisionId: string): void {
  vi.spyOn(Date, "now").mockReturnValue(Date.parse("2026-10-05T12:00:00Z"));
  render(
    <ControlCenterView
      client={client}
      projects={[fx.project({ id: "p1", name: "The Hermes" })]}
      bots={[fx.bot({ id: "ops", name: "DevOps", project_id: "p1" })]}
      connected
      canControl
      decisionId={decisionId}
      onToast={actionToastSpy()}
    />,
  );
}

describe("a release decision in Decisions", () => {
  it("shows the release review instead of the ruling composer", async () => {
    const asked = dfx.decision({
      id: "dec-rel-1",
      project_id: "p1",
      title: "Release R-2026-W41: ready for your ruling",
    });
    renderDecisions(decisions([asked], [release()]), "dec-rel-1");
    expect(await screen.findByRole("button", { name: "Approve 0.16.0" })).toBeInTheDocument();
    expect(screen.queryByRole("textbox", { name: /ruling/i })).not.toBeInTheDocument();
  });

  it("leaves other decisions as they are", async () => {
    const other = dfx.decision({ id: "d-other", project_id: "p1", title: "Which colour?" });
    const client = decisions([other], [release()]);
    renderDecisions(client, "d-other");
    await waitFor(() =>
      expect(client.requests.some((r) => r.body.type === "list_releases")).toBe(true),
    );
    expect(screen.queryByRole("button", { name: /Approve 0\.16\.0/ })).not.toBeInTheDocument();
  });
});
