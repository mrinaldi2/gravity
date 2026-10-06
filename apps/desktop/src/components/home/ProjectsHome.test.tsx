import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { FakeDaemon } from "../../test/fakeDaemon";
import * as fx from "../../test/fixtures";
import { HOME_NOW, overviewJson } from "../../test/homeFixtures";
import type { AddToast } from "../../app/useToasts";
import ProjectsHome from "./ProjectsHome";

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
      projects={[fx.project()]}
      bots={[fx.bot()]}
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
  it("ranks the projects and says why the first one needs you", async () => {
    renderHome(daemonWithOverview());
    const cards = await screen.findAllByTestId("project-card");
    expect(cards.map((card) => card.getAttribute("aria-label"))).toEqual([
      "The Hermes",
      "PhD",
      "Aurora Notes",
    ]);
    const first = within(cardAt(cards, 0));
    expect(first.getByText("#1")).toBeInTheDocument();
    expect(
      first.getByText("Needs you most: 1 release to test · 2 decisions · 1 Run card"),
    ).toBeInTheDocument();
    expect(first.getByText("◐ 0.17.0 ready for you to test")).toBeInTheDocument();
    expect(first.getByText("8 · 5 working")).toBeInTheDocument();
    expect(first.getByText("Project-first desktop UI")).toBeInTheDocument();
    expect(first.getByText(/0\.17\.0 is packaged/)).toBeInTheDocument();
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
    expect(within(card).getByText("#2")).toBeInTheDocument();
  });

  it("pins a project and reads the order again", async () => {
    const user = userEvent.setup();
    const daemon = daemonWithOverview();
    renderHome(daemon);
    const card = await screen.findByRole("article", { name: "Aurora Notes" });
    const before = overviewReads(daemon);
    await user.click(within(card).getByRole("button", { name: "Pin" }));
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
        name: "📌 Pinned",
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
    expect(within(card).queryByRole("button", { name: "Pin" })).not.toBeInTheDocument();
  });
});
