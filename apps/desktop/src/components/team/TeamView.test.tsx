import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { decodeOverview, decodeOwnerThreads } from "../../protocol/home";
import * as fx from "../../test/fixtures";
import { HOME_NOW, overviewJson, ownerThreadsJson } from "../../test/homeFixtures";
import TeamView from "./TeamView";

const BOTS = [
  fx.bot({ id: "b1", name: "Team Lead", state: "working" }),
  fx.bot({ id: "b2", name: "Desktop Dev", state: "waiting_for_user" }),
  fx.bot({ id: "b3", name: "Architect", state: "ready", description: "" }),
];

function renderTeam(onOpenBot = vi.fn<(botId: string) => void>(), canControl = true): void {
  const row = decodeOverview(overviewJson()).rows.find((r) => r.projectId === "p1") ?? null;
  render(
    <TeamView
      bots={BOTS}
      row={row}
      threads={decodeOwnerThreads(ownerThreadsJson()).threads}
      unread={{ b2: 1 }}
      failed={{}}
      activity={{}}
      leadBotId="b1"
      now={HOME_NOW}
      canControl={canControl}
      onOpenBot={onOpenBot}
      onCreateBot={vi.fn<() => void>()}
      onDeleteBot={vi.fn<(botId: string) => void>()}
    />,
  );
}

describe("TeamView", () => {
  it("shows each bot as a card with its state in words", () => {
    renderTeam();
    expect(screen.getByText("3 bots · 1 working")).toBeInTheDocument();
    const lead = within(screen.getByRole("article", { name: "Team Lead" }));
    expect(lead.getByText("◑ Working")).toBeInTheDocument();
    expect(lead.getByText(/^Lead/)).toBeInTheDocument();
    const dev = within(screen.getByRole("article", { name: "Desktop Dev" }));
    expect(dev.getByText("▲ Waiting for you")).toBeInTheDocument();
    expect(dev.getByText("1 new")).toBeInTheDocument();
    expect(dev.getByText(/Asked you/)).toBeInTheDocument();
    const idle = within(screen.getByRole("article", { name: "Architect" }));
    expect(idle.getByText("○ Idle")).toBeInTheDocument();
    expect(idle.getByText("No current item")).toBeInTheDocument();
  });

  it("says what a bot is doing from the board", () => {
    renderTeam();
    // b1 is the Doing item's assignee in the fixture.
    expect(
      within(screen.getByRole("article", { name: "Team Lead" })).getByText(
        "Lead · H-133 Project-first desktop UI",
      ),
    ).toBeInTheDocument();
  });

  it("opens a bot's page", async () => {
    const user = userEvent.setup();
    const onOpenBot = vi.fn<(botId: string) => void>();
    renderTeam(onOpenBot);
    await user.click(screen.getByRole("button", { name: "Open Desktop Dev" }));
    expect(onOpenBot).toHaveBeenCalledWith("b2");
  });

  it("offers a new bot only with control", () => {
    renderTeam(undefined, false);
    expect(screen.queryByRole("button", { name: "＋ New bot" })).not.toBeInTheDocument();
  });
});
