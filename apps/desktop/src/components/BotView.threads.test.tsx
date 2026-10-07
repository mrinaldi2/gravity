// One conversation with each bot (H-192, UX-033): on the bot's page, Chat is
// the owner thread the main chat shows, and the session's transcript is
// Activity.

import { fromJson } from "@bufbuild/protobuf";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { JsonValue } from "@bufbuild/protobuf";
import { ThreadMessageSchema } from "../protocol/gen/hermes/home/v1/home_pb";
import type { ThreadMessage } from "../protocol/gen/hermes/home/v1/home_pb";
import { text, turn } from "../test/chatFixtures";
import { FakeDaemon } from "../test/fakeDaemon";
import * as fx from "../test/fixtures";
import { ownerThreadJson } from "../test/homeFixtures";
import { actionToastSpy, botSpy, routinesSpy, stubLocalStorage } from "../test/spies";
import { ANSWER_HINT_MS, unansweredSince } from "./bot/BotChat";
import type { BotTab } from "./bot/BotTabs";
import BotView from "./BotView";

vi.mock("./TerminalPane", () => ({
  default: (): React.ReactElement => <div data-testid="terminal" />,
}));

/** A service with owner threads; `answers` adds the answer capture. */
function daemon(answers = true): FakeDaemon {
  const client = new FakeDaemon()
    .onRequest("list_routines", () => ({ type: "routines", req_id: "1", routines: [] }))
    .onRequest("owner_thread_get", () => ({
      type: "owner_thread",
      req_id: "1",
      owner_thread: ownerThreadJson(),
    }))
    .onRequest("owner_thread_read", () => ({
      type: "owner_thread_marked",
      req_id: "1",
      owner_thread_marked: {},
    }))
    .onRequest("send_user_message", () => ({ type: "message", req_id: "1", message: fx.message() }))
    .onRequest("list_chat", () => ({
      type: "chat",
      req_id: "2",
      bot_id: "b1",
      has_more: false,
      turns: [
        turn({
          id: "t1",
          items: [text("Checking.", "x1"), text("Green: 597 tests.", "x2")],
          answer_num: 13,
        }),
        turn({
          id: "t2",
          items: [{ type: "sent", id: "s1", to: "owner", msg_kind: "owner", body: "Shipped." }],
        }),
      ],
    }));
  client.capabilities = [
    "terminal_attach",
    "chat",
    "owner_threads",
    ...(answers ? ["owner_answers"] : []),
  ];
  return client;
}

function renderView(client: FakeDaemon, initialTab?: BotTab): void {
  const bot = fx.bot({ id: "b1", name: "Desktop Dev" });
  render(
    <BotView
      client={client}
      bot={bot}
      bots={[bot]}
      initialTab={initialTab}
      connected
      canControl
      onBotUpdated={botSpy()}
      onRoutinesChanged={routinesSpy()}
      onToast={actionToastSpy()}
    />,
  );
}

/** The bot page's tab bar: the Activity caption has a "Chat" link of its own. */
const tabBar = (): HTMLElement => document.querySelector<HTMLElement>("nav.tabs") as HTMLElement;
const tab = (name: string): HTMLElement => within(tabBar()).getByRole("button", { name });

describe("BotView, one conversation (H-192)", () => {
  beforeEach(() => {
    stubLocalStorage();
  });

  it("offers Reports, Chat, Activity and Terminal, and opens on Chat when asked", async () => {
    renderView(daemon(), "chat");
    const names = within(tabBar()).getAllByRole("button");
    expect(names.map((b) => b.textContent)).toEqual([
      "Reports",
      "Chat",
      "Activity",
      "Terminal",
      "Routines",
    ]);
    expect(tab("Chat")).toHaveClass("tab-active");
    // The owner thread, the same the main chat shows.
    expect(await screen.findByText("Should Resume now also restart the services?")).toBeVisible();
  });

  it("reads the thread in Chat, which marks it read everywhere", async () => {
    const client = daemon();
    renderView(client, "chat");
    await waitFor(() => {
      expect(client.requests.map((r) => r.body)).toContainEqual({
        type: "owner_thread_read",
        bot_id: "b1",
        up_to_num: 12,
      });
    });
  });

  it("sends from Chat to the same conversation", async () => {
    const client = daemon();
    renderView(client, "chat");
    await screen.findByText("Pushed 058a4f7.");
    // Activity stays mounted beside it with a composer of its own.
    const chat = document.querySelector<HTMLTextAreaElement>(".bot-chat textarea");
    expect(chat).not.toBeNull();
    await userEvent.type(chat as HTMLTextAreaElement, "Ship it{Enter}");
    await waitFor(() => {
      expect(client.requests.map((r) => r.body)).toContainEqual({
        type: "send_user_message",
        to_bot_id: "b1",
        body: "Ship it",
      });
    });
  });

  it("shows the work in Activity, with what reached Chat marked", async () => {
    renderView(daemon());
    await userEvent.click(tab("Activity"));
    expect(
      screen.getByText(/Everything Desktop Dev did: tasks, messages from other bots/),
    ).toBeVisible();
    expect(await screen.findByText("Green: 597 tests.")).toBeVisible();
    // The answer posted to the thread, and the note sent there.
    expect(screen.getByText("Sent you")).toBeVisible();
    expect(screen.getByText("Shipped.")).toBeVisible();
    expect(screen.getByText("Shows in Chat too.")).toBeVisible();
    await userEvent.click(screen.getByRole("button", { name: "In Chat" }));
    expect(tab("Chat")).toHaveClass("tab-active");
    // The caption's own link goes there too.
    await userEvent.click(tab("Activity"));
    const caption = screen.getByText(/Everything Desktop Dev did/);
    await userEvent.click(within(caption).getByRole("button", { name: "Chat" }));
    expect(tab("Chat")).toHaveClass("tab-active");
  });

  it("keeps an older service's Chat as the transcript, and says so", async () => {
    const client = daemon();
    client.capabilities = ["terminal_attach", "chat"];
    renderView(client);
    expect(screen.queryByRole("button", { name: "Activity" })).toBeNull();
    expect(
      screen.getByText(/This computer's Hermes service shows Chat and Activity together/),
    ).toBeVisible();
  });

  it("points to Activity when a message waits on a service without the capture", async () => {
    renderView(daemon(false), "chat");
    // The thread ends on a bot's message: nothing waits.
    await screen.findByText("Pushed 058a4f7.");
    expect(screen.queryByText(/may have answered in Activity/)).toBeNull();
  });
});

function message(fromOwner: boolean, at: string): ThreadMessage {
  const json: JsonValue = { num: "1", id: "m", from_owner: fromOwner, text: "?", at };
  return fromJson(ThreadMessageSchema, json);
}

describe("unansweredSince", () => {
  const at = Date.parse("2026-10-06T10:00:00Z");

  it("waits two minutes after the owner's last message", () => {
    const mine = message(true, "2026-10-06T10:00:00Z");
    expect(unansweredSince([mine], at + ANSWER_HINT_MS - 1)).toBeNull();
    expect(unansweredSince([mine], at + ANSWER_HINT_MS)).toBe(mine);
    expect(unansweredSince([message(false, "2026-10-06T10:00:00Z")], at + 600_000)).toBeNull();
    expect(unansweredSince([], at)).toBeNull();
  });
});
