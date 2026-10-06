import type { Story } from "@ladle/react";
import { decodeOverview, decodeOwnerThreads } from "../../protocol/home";
import { bot } from "../../test/fixtures";
import { HOME_NOW, overviewJson, ownerThreadsJson } from "../../test/homeFixtures";
import TeamView from "./TeamView";

const row = decodeOverview(overviewJson()).rows.find((r) => r.projectId === "p1") ?? null;
const noop = (): void => {};

/** Bots as cards: state in words, what each does, what it last sent you. */
export const Team: Story = () => (
  <div className="main" style={{ height: 520 }}>
    <TeamView
      bots={[
        bot({ id: "b1", name: "Team Lead", state: "working" }),
        bot({ id: "b2", name: "Desktop Dev", state: "waiting_for_user", description: "" }),
        bot({ id: "b3", name: "Architect", state: "ready", description: "Reviews designs" }),
        bot({ id: "b4", name: "Tester Win", state: "crashed", description: "Tests on win-pc" }),
      ]}
      row={row}
      threads={decodeOwnerThreads(ownerThreadsJson()).threads}
      unread={{ b2: 1 }}
      failed={{}}
      activity={{}}
      leadBotId="b1"
      now={HOME_NOW}
      canControl
      onOpenBot={noop}
      onCreateBot={noop}
      onDeleteBot={noop}
    />
  </div>
);
