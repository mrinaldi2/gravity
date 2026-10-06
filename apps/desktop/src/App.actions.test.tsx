import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import App from "./App";
import { backToTeam, botCard, openBotFromTeam, openTeam, waitForHome } from "./test/appNav";
import { FakeDaemon } from "./test/fakeDaemon";
import * as fx from "./test/fixtures";
import { stubLocalStorage } from "./test/spies";

let daemon: FakeDaemon;

vi.mock("./components/TerminalPane", () => ({
  default: (): React.ReactElement => <div data-testid="terminal" />,
}));
// `new DaemonClient(...)` yields the prepared double: a constructor that
// returns an object produces that object.
vi.mock("./protocol/client", () => ({
  DaemonClient: function DaemonClientDouble(): FakeDaemon {
    return daemon;
  },
}));
vi.mock("./token", () => ({ readClientToken: () => Promise.resolve("token") }));

function seedDaemon(): FakeDaemon {
  return new FakeDaemon()
    .onRequest("list_projects", () => ({ type: "projects", req_id: "1", projects: [fx.project()] }))
    .onRequest("list_bots", () => ({
      type: "bots",
      req_id: "1",
      bots: [fx.bot({ unread_count: 2 })],
    }))
    .onRequest("list_conversations", () => ({
      type: "conversations",
      req_id: "1",
      conversations: [fx.conversation()],
    }))
    .onRequest("list_deliveries", () => ({ type: "deliveries", req_id: "1", deliveries: [] }))
    .onRequest("list_routines", () => ({ type: "routines", req_id: "1", routines: [fx.routine()] }))
    .onRequest("list_messages", () => ({ type: "messages", req_id: "1", messages: [] }));
}

/** Renders the app and waits for the snapshot, on the projects home. */
async function renderHome(): Promise<void> {
  render(<App />);
  daemon.setStatus("connected");
  await waitForHome();
}

/** Renders the app, then opens alice from her project's Team, as the owner would. */
async function renderApp(): Promise<void> {
  await renderHome();
  await openTeam();
  await openBotFromTeam("alice");
}

describe("App actions", () => {
  beforeEach(() => {
    daemon = seedDaemon();
    daemon.status = "disconnected";
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("opens the project window on the Overview, then on the tab last used", async () => {
    const user = userEvent.setup();
    stubLocalStorage();
    await renderHome();

    await user.click(screen.getByRole("button", { name: "Open project" }));
    expect(screen.getByRole("tab", { name: "Overview", selected: true })).toBeInTheDocument();
    expect(
      await within(screen.getByRole("tabpanel")).findByText(
        /Loading the dashboard…|Couldn't load the dashboard|Needs you/,
      ),
    ).toBeInTheDocument();

    act(() => {
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "3", metaKey: true }));
    });
    expect(screen.getByRole("tab", { name: "Team", selected: true })).toBeInTheDocument();
    expect(botCard("alice")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "More ▾" }));
    await user.click(screen.getByRole("menuitem", { name: "Settings" }));
    expect(screen.getByRole("heading", { name: "Danger zone" })).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Settings ▾" }));
    await user.click(screen.getByRole("menuitem", { name: "Conversations" }));
    expect(screen.getByText("Conversations need a newer Hermes service.")).toBeInTheDocument();

    // Away to Projects (⌘0) and back: the window reopens where it was left.
    act(() => {
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "0", metaKey: true }));
    });
    await user.click(screen.getByRole("button", { name: "Open project" }));
    expect(screen.getByRole("button", { name: "Conversations ▾" })).toBeInTheDocument();
  });

  it("goes from a bot back to its project's Team", async () => {
    stubLocalStorage();
    await renderApp();
    await backToTeam();
    expect(screen.getByRole("tab", { name: "Team", selected: true })).toBeInTheDocument();
    await openBotFromTeam("alice");
    expect(screen.getByText("alice", { selector: ".view-title" })).toBeInTheDocument();
  });

  it("walks a fresh daemon through creating the first project", async () => {
    const user = userEvent.setup();
    daemon
      .onRequest("list_projects", () => ({ type: "projects", req_id: "1", projects: [] }))
      .onRequest("list_bots", () => ({ type: "bots", req_id: "1", bots: [] }))
      .onRequest("list_conversations", () => ({
        type: "conversations",
        req_id: "1",
        conversations: [],
      }))
      .onRequest("create_project", () => ({ type: "project", req_id: "1", project: fx.project() }))
      .onRequest("create_bot", () => ({ type: "bot", req_id: "1", bot: fx.bot() }));
    render(<App />);
    daemon.setStatus("connected");

    // A fresh daemon has no project card to wait for.
    await waitFor(() => {
      expect(screen.getByText("Welcome to The Hermes")).toBeInTheDocument();
    });
    // The post-create refresh must see what the daemon just made.
    daemon
      .onRequest("list_projects", () => ({
        type: "projects",
        req_id: "1",
        projects: [fx.project()],
      }))
      .onRequest("list_bots", () => ({ type: "bots", req_id: "1", bots: [fx.bot()] }));
    await user.type(screen.getByPlaceholderText("Project name"), "athens");
    await user.click(screen.getByRole("button", { name: "Create" }));

    await waitFor(() => {
      expect(
        daemon.requests.some(
          (request) => request.body.type === "create_project" && request.body.name === "athens",
        ),
      ).toBe(true);
    });
    // The first bot rides along and its view opens, not another empty pane.
    await waitFor(() => {
      expect(
        daemon.requests.some(
          (request) => request.body.type === "create_bot" && request.body.project_id === "p1",
        ),
      ).toBe(true);
    });
    expect(screen.getByTestId("terminal")).toBeInTheDocument();
  });

  it("creates a project from Projects and a bot from Team", async () => {
    const user = userEvent.setup();
    const newBot = fx.bot({ id: "b2", name: "New bot", dir_name: "new-bot" });
    daemon
      .onRequest("create_project", () => ({ type: "project", req_id: "1", project: fx.project() }))
      .onRequest("create_bot", () => ({ type: "bot", req_id: "1", bot: newBot }));
    await renderHome();

    // The refresh after creating must already list what the daemon made.
    daemon.onRequest("list_bots", () => ({
      type: "bots",
      req_id: "1",
      bots: [fx.bot(), newBot],
    }));
    await user.click(screen.getByRole("button", { name: "＋ New project" }));
    await user.type(screen.getByPlaceholderText("Project name"), "Beta");
    await user.click(screen.getByRole("button", { name: "Create" }));
    await waitFor(() => {
      expect(daemon.requests.some((r) => r.body.type === "create_project")).toBe(true);
    });
    // A new project comes with its first bot, whose page opens.
    await waitFor(() => {
      expect(screen.getByText("New bot", { selector: ".view-title" })).toBeInTheDocument();
    });

    await backToTeam();
    await user.click(screen.getByRole("button", { name: "＋ New bot" }));
    await waitFor(() => {
      expect(daemon.requests.filter((r) => r.body.type === "create_bot")).toHaveLength(2);
    });
    const creations = daemon.requests.filter((request) => request.body.type === "create_bot");
    expect(creations.map((request) => request.body)).toEqual([
      { type: "create_bot", project_id: "p1" },
      { type: "create_bot", project_id: "p1" },
    ]);
    expect(screen.getByText("New bot", { selector: ".view-title" })).toBeInTheDocument();
    expect(screen.getByTestId("terminal")).toBeInTheDocument();
  });

  it("deletes a bot from its Team card", async () => {
    const user = userEvent.setup();
    daemon.onRequest("delete_bot", () => ({ type: "ok", req_id: "1" }));
    await renderHome();
    await openTeam();

    await user.click(screen.getByRole("button", { name: "More for alice" }));
    await user.click(screen.getByRole("menuitem", { name: "Delete bot" }));
    await user.click(screen.getByRole("button", { name: "Delete bot" }));

    await waitFor(() => {
      expect(daemon.requests.some((r) => r.body.type === "delete_bot")).toBe(true);
    });
  });

  it("reports delete failures", async () => {
    const user = userEvent.setup();
    daemon.onRequest("delete_bot", () => {
      throw new Error("bot not found");
    });
    await renderHome();
    await openTeam();

    await user.click(screen.getByRole("button", { name: "More for alice" }));
    await user.click(screen.getByRole("menuitem", { name: "Delete bot" }));
    await user.click(screen.getByRole("button", { name: "Delete bot" }));

    await waitFor(() => {
      expect(screen.getByText("Delete bot failed")).toBeInTheDocument();
    });
  });

  it("reports create failures", async () => {
    const user = userEvent.setup();
    daemon.onRequest("create_project", () => {
      throw new Error("duplicate");
    });
    await renderHome();

    await user.click(screen.getByRole("button", { name: "＋ New project" }));
    await user.type(screen.getByPlaceholderText("Project name"), "Acme");
    await user.click(screen.getByRole("button", { name: "Create" }));

    await waitFor(() => {
      expect(screen.getByText("Create project failed")).toBeInTheDocument();
    });
  });

  it("opens the palette with ⌘K and navigates to a bot", async () => {
    const user = userEvent.setup();
    await renderApp();

    await user.keyboard("{Meta>}k{/Meta}");
    expect(screen.getByRole("dialog", { name: "Command palette" })).toBeInTheDocument();

    await user.click(screen.getByText("Diagnostics", { selector: ".overlay-label" }));
    await waitFor(() => {
      expect(screen.getByText("Deliveries")).toBeInTheDocument();
    });
  });

  it("closes the palette on a second ⌘K", async () => {
    const user = userEvent.setup();
    await renderApp();

    await user.keyboard("{Control>}k{/Control}");
    expect(screen.getByRole("dialog", { name: "Command palette" })).toBeInTheDocument();
    await user.keyboard("{Control>}k{/Control}");
    expect(screen.queryByRole("dialog", { name: "Command palette" })).not.toBeInTheDocument();
  });

  it("searches from the palette and opens the matching conversation", async () => {
    const user = userEvent.setup();
    daemon.onRequest("search", () => ({
      type: "search_results",
      req_id: "1",
      search_results: [fx.message({ body: "standup notes" })],
    }));
    await renderHome();

    await user.keyboard("{Meta>}k{/Meta}");
    await user.keyboard("notes");
    await user.click(screen.getByText("Search: notes"));
    await waitFor(() => {
      expect(screen.getByText("standup notes")).toBeInTheDocument();
    });

    await user.click(screen.getByText("standup notes"));
    await waitFor(() => {
      expect(screen.getByTestId("terminal")).toBeInTheDocument();
    });
  });

  it("warns when a search result points at a vanished conversation", async () => {
    const user = userEvent.setup();
    daemon.onRequest("search", () => ({
      type: "search_results",
      req_id: "1",
      search_results: [fx.message({ conversation_id: "ghost", body: "orphan" })],
    }));
    await renderHome();

    await user.keyboard("{Meta>}k{/Meta}");
    await user.keyboard("orphan");
    await user.click(screen.getByText("Search: orphan"));
    await user.click(await screen.findByText("orphan"));

    await waitFor(() => {
      expect(screen.getByText("Conversation not found")).toBeInTheDocument();
    });
  });

  it("changes the daemon endpoint from the rail's connection dot", async () => {
    const user = userEvent.setup();
    await renderHome();

    await user.click(screen.getByRole("button", { name: /^Hermes service:/ }));
    const host = screen.getByLabelText("Computer host");
    await user.clear(host);
    await user.type(host, "mini");
    await user.click(screen.getByRole("button", { name: "Connect" }));

    await waitFor(() => {
      expect(daemon.getEndpoint()).toEqual({ host: "mini", port: 7777 });
    });
  });

  it("opens a bot from the command palette and offers no lifecycle entries", async () => {
    const user = userEvent.setup();
    await renderHome();

    await user.keyboard("{Meta>}k{/Meta}");
    expect(screen.queryByText("Stop alice")).not.toBeInTheDocument();
    expect(screen.queryByText("Start alice")).not.toBeInTheDocument();

    await user.click(screen.getByText("alice", { selector: ".overlay-label" }));
    await waitFor(() => {
      expect(screen.getByText("alice", { selector: ".view-title" })).toBeInTheDocument();
    });
    expect(screen.queryByRole("button", { name: "Stop" })).not.toBeInTheDocument();
  });
});
