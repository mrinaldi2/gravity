import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { ProjectTab } from "../../app/selection";
import { decodeOverview } from "../../protocol/home";
import * as fx from "../../test/fixtures";
import { overviewJson } from "../../test/homeFixtures";
import ProjectWindow from "./ProjectWindow";

function renderWindow(tab: ProjectTab = "overview", onHome?: () => void) {
  const onSelectTab = vi.fn<(tab: ProjectTab) => void>();
  const row = decodeOverview(overviewJson()).rows.find((r) => r.projectId === "p1") ?? null;
  render(
    <ProjectWindow
      project={fx.project()}
      botCount={2}
      tab={tab}
      row={row}
      onHome={onHome}
      onSelectTab={onSelectTab}
    >
      <p>tab content</p>
    </ProjectWindow>,
  );
  return onSelectTab;
}

describe("ProjectWindow", () => {
  it("names the project, what needs you and its release", () => {
    renderWindow();
    expect(screen.getByRole("heading", { name: "Acme" })).toBeInTheDocument();
    expect(screen.getByText("2 bots")).toBeInTheDocument();
    expect(screen.getByText("▲ 4 need you")).toBeInTheDocument();
    expect(screen.getByText("◐ 0.17.0 ready for you to test")).toBeInTheDocument();
  });

  it("goes back to Projects from the crumb", async () => {
    const user = userEvent.setup();
    const onHome = vi.fn<() => void>();
    renderWindow("overview", onHome);
    await user.click(screen.getByRole("button", { name: "Projects ›" }));
    expect(onHome).toHaveBeenCalled();
  });

  it("opens Needs you and the release from the header pills", async () => {
    const user = userEvent.setup();
    const onSelectTab = renderWindow("team");
    await user.click(screen.getByRole("button", { name: "▲ 4 need you" }));
    await user.click(screen.getByRole("button", { name: "◐ 0.17.0 ready for you to test" }));
    expect(onSelectTab.mock.calls.map((call) => call[0])).toEqual(["overview", "releases"]);
  });

  it("offers the five tabs in order, then More", () => {
    renderWindow("releases");
    const tabs = screen.getAllByRole("tab");
    expect(tabs.map((tab) => tab.textContent)).toEqual([
      "Overview",
      "Board",
      "Team",
      "Releases",
      "Meetings",
    ]);
    expect(screen.getByRole("tab", { selected: true })).toHaveTextContent("Releases");
    expect(screen.getByRole("tablist", { name: "Project" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "More ▾" })).toBeInTheDocument();
  });

  it("opens Conversations and Settings from More, which then names the tab", async () => {
    const user = userEvent.setup();
    const onSelectTab = renderWindow("settings");
    expect(screen.getByRole("tabpanel", { name: "Settings ▾" })).toHaveTextContent("tab content");
    await user.click(screen.getByRole("button", { name: "Settings ▾" }));
    await user.click(screen.getByRole("menuitem", { name: "Conversations" }));
    expect(onSelectTab).toHaveBeenCalledWith("conversations");
    // Away from the bar, the first tab keeps the one tab stop.
    const stops = screen.getAllByRole("tab").filter((tab) => tab.tabIndex === 0);
    expect(stops.map((tab) => tab.textContent)).toEqual(["Overview"]);
  });

  it("labels the panel with the active tab", () => {
    renderWindow("board");
    expect(screen.getByRole("tabpanel", { name: "Board" })).toHaveTextContent("tab content");
  });

  it("keeps one tab stop, on the active tab", () => {
    renderWindow("meetings");
    const stops = screen.getAllByRole("tab").filter((tab) => tab.tabIndex === 0);
    expect(stops.map((tab) => tab.textContent)).toEqual(["Meetings"]);
  });

  it("selects a tab on click", async () => {
    const user = userEvent.setup();
    const onSelectTab = renderWindow();
    await user.click(screen.getByRole("tab", { name: "Team" }));
    expect(onSelectTab).toHaveBeenCalledWith("team");
  });

  it("moves between tabs with the arrow keys, Home and End, wrapping at the ends", () => {
    const onSelectTab = renderWindow("overview");
    const overview = screen.getByRole("tab", { name: "Overview" });

    fireEvent.keyDown(overview, { key: "ArrowRight" });
    fireEvent.keyDown(overview, { key: "ArrowLeft" });
    fireEvent.keyDown(overview, { key: "End" });
    fireEvent.keyDown(overview, { key: "Home" });
    fireEvent.keyDown(overview, { key: "Enter" });

    expect(onSelectTab.mock.calls.map((call) => call[0])).toEqual([
      "board",
      "meetings",
      "meetings",
      "overview",
    ]);
    expect(screen.getByRole("tab", { name: "Overview" })).toHaveFocus();
  });

  it("switches tabs with ⌘1 to ⌘5", () => {
    const onSelectTab = renderWindow();

    fireEvent.keyDown(window, { key: "3", metaKey: true });
    fireEvent.keyDown(window, { key: "5", ctrlKey: true });
    fireEvent.keyDown(window, { key: "6", metaKey: true });
    fireEvent.keyDown(window, { key: "2" });
    fireEvent.keyDown(window, { key: "2", metaKey: true, shiftKey: true });

    expect(onSelectTab.mock.calls.map((call) => call[0])).toEqual(["team", "meetings"]);
  });

  it("shows each shortcut on its tab", () => {
    renderWindow();
    expect(screen.getByRole("tab", { name: "Meetings" })).toHaveAttribute("title", "Meetings (⌘5)");
    expect(screen.getByRole("tab", { name: "Meetings" })).toHaveAttribute(
      "aria-keyshortcuts",
      "Meta+5",
    );
  });
});
