import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { decodeOverview, decodeOwnerThreads } from "../../protocol/home";
import * as fx from "../../test/fixtures";
import { HOME_NOW, overviewJson, ownerThreadsJson } from "../../test/homeFixtures";
import type { MeetingSummary } from "../../protocol/meetings";
import { meetingDetail } from "../../test/meetingFixtures";
import FromTheTeam from "./FromTheTeam";
import type { ReplyTo } from "./FromTheTeam";

const BOTS = [
  fx.bot({ id: "b1", name: "Team Lead" }),
  fx.bot({ id: "b2", name: "Desktop Dev" }),
  fx.bot({ id: "b3", name: "Scrum Master" }),
];

function renderFeed(
  onReply: ReplyTo,
  onOpenMeetings = vi.fn<() => void>(),
  summaryMeeting: MeetingSummary | null = null,
): void {
  const row = decodeOverview(overviewJson()).rows.find((r) => r.projectId === "p1") ?? null;
  render(
    <FromTheTeam
      row={row}
      threads={decodeOwnerThreads(ownerThreadsJson()).threads}
      bots={BOTS}
      leadBotId="b1"
      now={HOME_NOW}
      onReply={onReply}
      onOpenMeetings={onOpenMeetings}
      summaryMeeting={summaryMeeting}
    />,
  );
}

describe("FromTheTeam", () => {
  it("lists the open question first, then the newest reports", () => {
    renderFeed(vi.fn<ReplyTo>());
    const rows = screen.getAllByRole("listitem");
    expect(rows.map((row) => row.querySelector(".dash-row-title")?.textContent)).toEqual([
      expect.stringMatching(/^Desktop Dev · asks you · /),
      expect.stringMatching(/^Team Lead · sent you · /),
      expect.stringMatching(/^Meeting minutes · Team Lead · /),
    ]);
  });

  it("names the meeting a summary came from and signs it with its facilitator", async () => {
    const user = userEvent.setup();
    const onReply = vi.fn<ReplyTo>();
    renderFeed(onReply, undefined, { ...meetingDetail("m1"), type: "standup", facilitator: "b3" });
    expect(screen.getByText(/^Stand-up minutes · Scrum Master · /)).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Reply to Scrum Master" }));
    expect(onReply).toHaveBeenCalledWith("b3", "0.17.0 is packaged; installs wait on your ruling.");
  });

  it("replies to the bot, quoting what it said", async () => {
    const user = userEvent.setup();
    const onReply = vi.fn<ReplyTo>();
    renderFeed(onReply);
    await user.click(screen.getByRole("button", { name: "Reply to Desktop Dev" }));
    expect(onReply).toHaveBeenCalledWith("b2", "Should Resume now also restart the services?");
  });

  it("opens the meeting a summary came from", async () => {
    const user = userEvent.setup();
    const onOpenMeetings = vi.fn<() => void>();
    renderFeed(vi.fn<ReplyTo>(), onOpenMeetings);
    const summary = screen.getByText(/0\.17\.0 is packaged/).closest("li");
    await user.click(within(summary as HTMLElement).getByRole("button", { name: "Open" }));
    expect(onOpenMeetings).toHaveBeenCalled();
  });

  it("says so when the team has sent nothing", () => {
    render(
      <FromTheTeam
        row={null}
        threads={[]}
        bots={BOTS}
        leadBotId="b1"
        now={HOME_NOW}
        onReply={vi.fn<ReplyTo>()}
        onOpenMeetings={vi.fn<() => void>()}
      />,
    );
    expect(screen.getByText(/Nothing from the team yet/)).toBeInTheDocument();
  });
});
