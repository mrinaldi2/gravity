// The main chat's way to a bot's page (H-192, UX-034): "Open <bot>" always
// lands on the bot's Chat, even when that page is already open from the main
// chat and the owner has moved to Activity.

import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import App from "./App";
import { openBotFromTeam, openTeam, waitForHome } from "./test/appNav";
import { FakeDaemon } from "./test/fakeDaemon";
import * as fx from "./test/fixtures";
import { stubLocalStorage } from "./test/spies";

let daemon: FakeDaemon;

vi.mock("./components/TerminalPane", () => ({
  default: (): React.ReactElement => <div data-testid="terminal" />,
}));
vi.mock("./protocol/client", () => ({
  DaemonClient: function DaemonClientDouble(): FakeDaemon {
    return daemon;
  },
}));
vi.mock("./token", () => ({ readClientToken: () => Promise.resolve("token") }));

/** A service with owner threads, one project and alice in it. */
function threadsDaemon(): FakeDaemon {
  const fake = new FakeDaemon()
    .onRequest("list_projects", () => ({ type: "projects", req_id: "1", projects: [fx.project()] }))
    .onRequest("list_bots", () => ({ type: "bots", req_id: "1", bots: [fx.bot()] }))
    .onRequest("list_conversations", () => ({
      type: "conversations",
      req_id: "1",
      conversations: [fx.conversation()],
    }))
    .onRequest("list_deliveries", () => ({ type: "deliveries", req_id: "1", deliveries: [] }))
    .onRequest("list_routines", () => ({ type: "routines", req_id: "1", routines: [] }))
    .onRequest("list_messages", () => ({ type: "messages", req_id: "1", messages: [] }))
    .onRequest("owner_thread_get", () => ({
      type: "owner_thread",
      req_id: "1",
      owner_thread: { bot: { daemon_id: "d", bot_id: "b1", name: "alice" }, project_id: "p1" },
    }))
    .onRequest("owner_threads", () => ({ type: "owner_threads", req_id: "1", owner_threads: {} }))
    .onRequest("list_chat", () => ({
      type: "chat",
      req_id: "1",
      bot_id: "b1",
      has_more: false,
      turns: [],
    }));
  fake.capabilities = [...fake.capabilities, "chat", "owner_threads"];
  return fake;
}

const open = (): HTMLElement => screen.getByRole("button", { name: "Open alice's page" });

/** A tab of the bot page: Activity's caption has a "Chat" link of its own. */
function tab(name: string): HTMLElement {
  const bar = document.querySelector<HTMLElement>("nav.tabs");
  if (bar === null) {
    throw new Error("no bot tabs on screen");
  }
  return within(bar).getByRole("button", { name });
}

describe("App, Open <bot> from the main chat", () => {
  it("lands on the bot's Chat at every press", async () => {
    const user = userEvent.setup();
    daemon = threadsDaemon();
    daemon.status = "disconnected";
    stubLocalStorage();
    render(<App />);
    daemon.setStatus("connected");
    await waitForHome();
    await openTeam();
    await openBotFromTeam("alice");
    await user.keyboard("{Meta>}j{/Meta}");

    await user.click(open());
    expect(tab("Chat")).toHaveClass("tab-active");
    await user.click(tab("Activity"));
    expect(tab("Activity")).toHaveClass("tab-active");
    // The page is already {alice, Chat}: the press still takes it there.
    await user.click(open());
    expect(tab("Chat")).toHaveClass("tab-active");
  });
});
