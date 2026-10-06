import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { FakeDaemon } from "../../test/fakeDaemon";
import * as fx from "../../test/fixtures";
import { HOME_NOW, overviewJson, ownerThreadJson } from "../../test/homeFixtures";
import ReportsPane from "./ReportsPane";

function daemon(): FakeDaemon {
  const fake = new FakeDaemon()
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
  fake.capabilities = [...fake.capabilities, "owner_threads", "projects_overview"];
  return fake;
}

describe("ReportsPane", () => {
  it("shows the latest report and the open question, each with Reply", async () => {
    const user = userEvent.setup();
    const onReply = vi.fn<(quote: string) => void>();
    const bot = fx.bot({ id: "b2", name: "Desktop Dev", project_id: "p1" });
    render(<ReportsPane client={daemon()} bot={bot} connected onReply={onReply} now={HOME_NOW} />);
    const asked = (await screen.findByRole("heading", { name: "Asked you · 1" })).closest(
      "section",
    ) as HTMLElement;
    expect(within(asked).getByText(/restart the services/)).toBeInTheDocument();
    const latest = screen
      .getByRole("heading", { name: "Latest report" })
      .closest("section") as HTMLElement;
    expect(within(latest).getByText(/restart the services/)).toBeInTheDocument();
    await user.click(within(asked).getByRole("button", { name: "Reply" }));
    expect(onReply).toHaveBeenCalledWith("Should Resume now also restart the services?");
  });

  it("says what the bot is doing on the board", async () => {
    const bot = fx.bot({ id: "b1", name: "Team Lead", project_id: "p1" });
    render(
      <ReportsPane
        client={daemon()}
        bot={bot}
        connected
        onReply={vi.fn<(quote: string) => void>()}
        now={HOME_NOW}
      />,
    );
    expect(await screen.findByText("Project-first desktop UI")).toBeInTheDocument();
  });
});
