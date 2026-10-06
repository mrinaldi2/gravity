import type { Story } from "@ladle/react";
import { FakeDaemon } from "../../test/fakeDaemon";
import { bot, project } from "../../test/fixtures";
import { meetingDetail, meetingListReply } from "../../test/meetingFixtures";
import MeetingsView from "./MeetingsView";

/** The list beside the newest held meeting's minutes. */
export const Minutes: Story = () => {
  const client = new FakeDaemon()
    .onRequest("meeting_list", () => meetingListReply())
    .onRequest("meeting_get", (body) => ({
      type: "meeting",
      req_id: "1",
      meeting: meetingDetail(body.type === "meeting_get" ? body.meeting_id : ""),
    }));
  return (
    <div className="main" style={{ height: 520 }}>
      <MeetingsView
        client={client}
        project={project()}
        bots={[bot({ id: "b1", name: "Scrum Master" })]}
        connected
      />
    </div>
  );
};
