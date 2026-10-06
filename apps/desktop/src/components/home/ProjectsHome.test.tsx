import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { FakeDaemon } from "../../test/fakeDaemon";
import * as fx from "../../test/fixtures";
import { HOME_NOW, overviewJson } from "../../test/homeFixtures";
import type { AddToast } from "../../app/useToasts";
import ProjectsHome from "./ProjectsHome";
import { OVERVIEW_WAIT_MS } from "./useProjectsOverview";

function cardAt(cards: readonly HTMLElement[], index: number): HTMLElement {
  const card = cards[index];
  if (card === undefined) {
    throw new Error(`no card ${index}`);
  }
  return card;
}

function daemonWithOverview(): FakeDaemon {
  const daemon = new FakeDaemon()
    .onRequest("projects_overview", () => ({
      type: "projects_overview",
      req_id: "1",
      overview: overviewJson(),
    }))
    .onRequest("project_pin", () => ({
      type: "project_pinned",
      req_id: "1",
      project_id: "p3",
      pinned: true,
    }));
  daemon.capabilities = [...daemon.capabilities, "projects_overview"];
  return daemon;
}

function renderHome(
  daemon: FakeDaemon,
  onOpenProject = vi.fn<(projectId: string) => void>(),
): void {
  render(
    <ProjectsHome
      client={daemon}
      projects={[fx.project({ id: "p1", lead_bot_id: "b1" })]}
      bots={[fx.bot({ id: "b1", name: "Team Lead" })]}
      connected
      canControl
      addToast={vi.fn<AddToast>()}
      onOpenProject={onOpenProject}
      onCreateProject={vi.fn<(name: string) => Promise<void>>(async () => {})}
      now={HOME_NOW}
    />,
  );
}

function overviewReads(daemon: FakeDaemon): number {
  return daemon.requests.filter((request) => request.body.type === "projects_overview").length;
}

describe("ProjectsHome", () => {
  it("puts pinned projects first and numbers the rest by need", async () => {
    renderHome(daemonWithOverview());
    const cards = await screen.findAllByTestId("project-card");
    expect(cards.map((card) => card.getAttribute("aria-label"))).toEqual([
      "PhD",
      "The Hermes",
      "Aurora Notes",
    ]);
    const pinned = within(cardAt(cards, 0));
    expect(pinned.getByLabelText("Pinned")).toBeInTheDocument();
    expect(pinned.queryByText(/^#/)).not.toBeInTheDocument();
    expect(pinned.getByText("1 decision")).toBeInTheDocument();
    // The outline follows the highest score, not the pin.
    expect(cardAt(cards, 0)).not.toHaveClass("home-card-first");
    expect(cardAt(cards, 1)).toHaveClass("home-card-first");
    const top = within(cardAt(cards, 1));
    expect(top.getByText("#1")).toBeInTheDocument();
    expect(
      top.getByText("Needs you most: 1 release to test · 2 decisions · 1 Run card"),
    ).toBeInTheDocument();
    expect(top.getByText("◐ 0.17.0 ready for you to test")).toBeInTheDocument();
    expect(top.getByText("8 · 5 working")).toBeInTheDocument();
    expect(top.getByText("Doing")).toBeInTheDocument();
    expect(top.getByText("Project-first desktop UI · Desktop Dev")).toBeInTheDocument();
    expect(top.getByText(/Team Lead · .*:/)).toBeInTheDocument();
    expect(top.getByText(/0\.17\.0 is packaged/)).toBeInTheDocument();
    const last = within(cardAt(cards, 2));
    expect(last.getByLabelText("Nothing needs you")).toBeInTheDocument();
    expect(last.getByText("Last activity 3 days ago")).toBeInTheDocument();
    expect(last.getByText("○ No release yet")).toBeInTheDocument();
    expect(screen.getByText("3 projects · 2 computers")).toBeInTheDocument();
  });

  it("says when a computer is away, with when it was last seen", async () => {
    renderHome(daemonWithOverview());
    const card = await screen.findByRole("article", { name: "PhD" });
    expect(within(card).getByText(/imac is offline · last seen/)).toBeInTheDocument();
  });

  it("pins a project and reads the order again", async () => {
    const user = userEvent.setup();
    const daemon = daemonWithOverview();
    renderHome(daemon);
    const card = await screen.findByRole("article", { name: "Aurora Notes" });
    const before = overviewReads(daemon);
    await user.click(within(card).getByRole("button", { name: "Pin Aurora Notes to the top" }));
    expect(daemon.requests.map((request) => request.body)).toContainEqual({
      type: "project_pin",
      project_id: "p3",
      pinned: true,
    });
    await waitFor(() => {
      expect(overviewReads(daemon)).toBeGreaterThan(before);
    });
    expect(
      within(screen.getByRole("article", { name: "PhD" })).getByRole("button", {
        name: "Unpin PhD",
        pressed: true,
      }),
    ).toBeInTheDocument();
  });

  it("reads again when the service says a project changed", async () => {
    const daemon = daemonWithOverview();
    renderHome(daemon);
    await screen.findAllByTestId("project-card");
    const before = overviewReads(daemon);
    act(() => {
      daemon.emit("projects_overview_changed", {
        type: "projects_overview_changed",
        project_ids: ["p1"],
      });
    });
    await waitFor(() => {
      expect(overviewReads(daemon)).toBe(before + 1);
    });
  });

  it("opens a project", async () => {
    const user = userEvent.setup();
    const onOpenProject = vi.fn<(projectId: string) => void>();
    renderHome(daemonWithOverview(), onOpenProject);
    const card = await screen.findByRole("article", { name: "The Hermes" });
    await user.click(within(card).getByRole("button", { name: "Open project" }));
    expect(onOpenProject).toHaveBeenCalledWith("p1");
  });

  it("shows plain rows from an older service, without counts", () => {
    renderHome(new FakeDaemon());
    const card = screen.getByRole("article", { name: "Acme" });
    expect(within(card).getByText("Update needed for full info")).toBeInTheDocument();
    expect(within(card).queryByRole("button", { name: /^Pin/ })).not.toBeInTheDocument();
  });

  // H-167: the 0.17.0 home stayed empty when the overview failed or never
  // answered. Every project stays on the home, and opens.
  it("shows the plain list, and says why, when the overview fails", async () => {
    const user = userEvent.setup();
    const daemon = daemonWithOverview().onRequest("projects_overview", () => {
      throw new Error("the service failed on 'projects_overview'");
    });
    const onOpenProject = vi.fn<(projectId: string) => void>();
    renderHome(daemon, onOpenProject);
    expect(await screen.findByRole("alert")).toHaveTextContent(
      /Couldn't load what each project needs \(the service failed on 'projects_overview'\)/,
    );
    expect(screen.queryByText(/older Hermes service/)).not.toBeInTheDocument();
    const card = screen.getByRole("article", { name: "Acme" });
    await user.click(within(card).getByRole("button", { name: "Open project" }));
    expect(onOpenProject).toHaveBeenCalledWith("p1");
  });

  it("shows the plain list when the overview doesn't answer in time", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    try {
      const daemon = daemonWithOverview();
      const request = daemon.request.bind(daemon);
      vi.spyOn(daemon, "request").mockImplementation((body, expect) =>
        body.type === "projects_overview" ? new Promise(() => {}) : request(body, expect),
      );
      renderHome(daemon);
      expect(screen.queryByRole("article", { name: "Acme" })).not.toBeInTheDocument();
      await act(async () => {
        await vi.advanceTimersByTimeAsync(OVERVIEW_WAIT_MS);
      });
      expect(screen.getByRole("alert")).toHaveTextContent(/didn't answer in time/);
      expect(screen.getByRole("article", { name: "Acme" })).toBeInTheDocument();
    } finally {
      vi.useRealTimers();
    }
  });
});
