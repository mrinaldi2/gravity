import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { DaemonError } from "../../protocol/connection";
import { FakeDaemon } from "../../test/fakeDaemon";
import * as fx from "../../test/fixtures";
import { meetingDetail, meetingListReply } from "../../test/meetingFixtures";
import MeetingsView from "./MeetingsView";

const BOTS = [fx.bot({ id: "b1", name: "Scrum Master" })];

describe("MeetingsView", () => {
  it("lists the series and meetings, and opens the newest held one's minutes", async () => {
    const daemon = new FakeDaemon()
      .onRequest("meeting_list", () => meetingListReply())
      .onRequest("meeting_get", (body) => ({
        type: "meeting",
        req_id: "1",
        meeting: meetingDetail(body.type === "meeting_get" ? body.meeting_id : ""),
      }));
    render(<MeetingsView client={daemon} project={fx.project()} bots={BOTS} connected />);
    expect(await screen.findByText("Stand-up")).toBeInTheDocument();
    expect(await screen.findByText(/Summary \(Scrum Master\):/)).toBeInTheDocument();
    expect(screen.getByText(/Verify is full/)).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Blockers" })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Action items" })).toBeInTheDocument();
    expect(screen.getByText("Collecting · 4 of 6 contributed")).toBeInTheDocument();
    expect(screen.getByText("Split H-014")).toBeInTheDocument();
  });

  it("opens another meeting", async () => {
    const user = userEvent.setup();
    const daemon = new FakeDaemon()
      .onRequest("meeting_list", () => meetingListReply())
      .onRequest("meeting_get", (body) => ({
        type: "meeting",
        req_id: "1",
        meeting: meetingDetail(body.type === "meeting_get" ? body.meeting_id : ""),
      }));
    render(<MeetingsView client={daemon} project={fx.project()} bots={BOTS} connected />);
    await user.click(await screen.findByRole("button", { name: /Retro W41/ }));
    expect(
      daemon.requests.some((r) => r.body.type === "meeting_get" && r.body.meeting_id === "m2"),
    ).toBe(true);
    expect(await screen.findByText(/Collecting: 4 of 6/)).toBeInTheDocument();
  });

  it("says where meetings are kept when the board lives elsewhere", async () => {
    const daemon = new FakeDaemon().onRequest("meeting_list", () => {
      throw new DaemonError(
        "forbidden",
        "This project's meetings are kept on mac, which holds its board.",
      );
    });
    render(<MeetingsView client={daemon} project={fx.project()} bots={BOTS} connected />);
    expect(await screen.findByText(/kept on mac/)).toBeInTheDocument();
  });
});
