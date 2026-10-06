import type { Story } from "@ladle/react";
import { FakeDaemon } from "../../test/fakeDaemon";
import { bot } from "../../test/fixtures";
import { HOME_NOW, overviewJson, ownerThreadJson } from "../../test/homeFixtures";
import ReportsPane from "./ReportsPane";

/** Where a bot's page opens: doing now, its latest report, its open question. */
export const Reports: Story = () => {
  const client = new FakeDaemon()
    .onRequest("owner_thread_get", () => ({
      type: "owner_thread",
      req_id: "1",
      owner_thread: ownerThreadJson(),
    }))
    .onRequest("projects_overview", () => ({
      type: "projects_overview",
      req_id: "1",
      overview: overviewJson(),
    }));
  client.capabilities = [...client.capabilities, "owner_threads", "projects_overview"];
  return (
    <div className="main" style={{ height: 480 }}>
      <ReportsPane
        client={client}
        bot={bot({ id: "b1", name: "Team Lead", project_id: "p1" })}
        connected
        onReply={() => {}}
        now={HOME_NOW}
      />
    </div>
  );
};
