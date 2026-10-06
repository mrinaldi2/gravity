import type { Story } from "@ladle/react";
import { decodeOverview, decodeOwnerThreads } from "../../protocol/home";
import { bot } from "../../test/fixtures";
import { HOME_NOW, overviewJson, ownerThreadsJson } from "../../test/homeFixtures";
import FromTheTeam from "./FromTheTeam";

const row = decodeOverview(overviewJson()).rows.find((r) => r.projectId === "p1") ?? null;
const noop = (): void => {};
const BOTS = [bot({ id: "b1", name: "Team Lead" }), bot({ id: "b2", name: "Desktop Dev" })];

/** The open question first, then the newest reports, each with Reply. */
export const Reports: Story = () => (
  <div className="dash" style={{ width: 760, padding: 16 }}>
    <div className="dash-grid">
      <FromTheTeam
        row={row}
        threads={decodeOwnerThreads(ownerThreadsJson()).threads}
        bots={BOTS}
        leadBotId="b1"
        now={HOME_NOW}
        onReply={noop}
        onOpenMeetings={noop}
      />
    </div>
  </div>
);
