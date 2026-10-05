import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { ProjectTab } from "../../app/selection";
import * as fx from "../../test/fixtures";
import ComingSoon from "./ComingSoon";
import ProjectWindow from "./ProjectWindow";

function renderWindow(tab: ProjectTab = "dashboard") {
  const onSelectTab = vi.fn<(tab: ProjectTab) => void>();
  render(
    <ProjectWindow project={fx.project()} botCount={2} tab={tab} onSelectTab={onSelectTab}>
      <p>tab content</p>
    </ProjectWindow>,
  );
  return onSelectTab;
}

describe("ProjectWindow", () => {
  it("names the project and its bot count", () => {
    renderWindow();
    expect(screen.getByRole("heading", { name: "Acme" })).toBeInTheDocument();
    expect(screen.getByText("2 bots")).toBeInTheDocument();
  });

  it("offers the six tabs in order, with the active one selected", () => {
    renderWindow("releases");
    const tabs = screen.getAllByRole("tab");
    expect(tabs.map((tab) => tab.textContent)).toEqual([
      "Dashboard",
      "Board",
      "Releases",
      "Meetings",
      "Conversations",
      "Settings",
    ]);
    expect(screen.getByRole("tab", { selected: true })).toHaveTextContent("Releases");
    expect(screen.getByRole("tablist", { name: "Project" })).toBeInTheDocument();
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
    await user.click(screen.getByRole("tab", { name: "Conversations" }));
    expect(onSelectTab).toHaveBeenCalledWith("conversations");
  });

  it("moves between tabs with the arrow keys, Home and End, wrapping at the ends", () => {
    const onSelectTab = renderWindow("dashboard");
    const dashboard = screen.getByRole("tab", { name: "Dashboard" });

    fireEvent.keyDown(dashboard, { key: "ArrowRight" });
    fireEvent.keyDown(dashboard, { key: "ArrowLeft" });
    fireEvent.keyDown(dashboard, { key: "End" });
    fireEvent.keyDown(dashboard, { key: "Home" });
    fireEvent.keyDown(dashboard, { key: "Enter" });

    expect(onSelectTab.mock.calls.map((call) => call[0])).toEqual([
      "board",
      "settings",
      "settings",
      "dashboard",
    ]);
    expect(screen.getByRole("tab", { name: "Dashboard" })).toHaveFocus();
  });

  it("switches tabs with ⌘1 to ⌘6", () => {
    const onSelectTab = renderWindow();

    fireEvent.keyDown(window, { key: "3", metaKey: true });
    fireEvent.keyDown(window, { key: "6", ctrlKey: true });
    fireEvent.keyDown(window, { key: "7", metaKey: true });
    fireEvent.keyDown(window, { key: "2" });
    fireEvent.keyDown(window, { key: "2", metaKey: true, shiftKey: true });

    expect(onSelectTab.mock.calls.map((call) => call[0])).toEqual(["releases", "settings"]);
  });

  it("shows each shortcut on its tab", () => {
    renderWindow();
    expect(screen.getByRole("tab", { name: "Meetings" })).toHaveAttribute("title", "Meetings (⌘4)");
    expect(screen.getByRole("tab", { name: "Meetings" })).toHaveAttribute(
      "aria-keyshortcuts",
      "Meta+4",
    );
  });
});

describe("ComingSoon", () => {
  it("says what the tab will hold", () => {
    render(<ComingSoon tab="meetings" />);
    expect(screen.getByRole("heading", { name: "Meetings is coming soon" })).toBeInTheDocument();
    expect(screen.getByText(/Stand-ups, demos and retros/)).toBeInTheDocument();
  });
});
