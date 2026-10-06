import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useEffect } from "react";
import type { ReactElement } from "react";
import { describe, expect, it, vi } from "vitest";
import type { AddToast } from "../../app/useToasts";
import { decodeOwnerThreads } from "../../protocol/home";
import { FakeDaemon } from "../../test/fakeDaemon";
import * as fx from "../../test/fixtures";
import { HOME_NOW, ownerThreadJson, ownerThreadsJson } from "../../test/homeFixtures";
import MainChat from "./MainChat";
import { useMainChat } from "./useMainChat";

const BOTS = [fx.bot({ id: "b1", name: "Team Lead" }), fx.bot({ id: "b2", name: "Desktop Dev" })];

function daemon(): FakeDaemon {
  const fake = new FakeDaemon()
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
    .onRequest("send_user_message", () => ({
      type: "message",
      req_id: "1",
      message: fx.message(),
    }));
  fake.capabilities = [...fake.capabilities, "owner_threads"];
  return fake;
}

const leadFallback = (): string => "b1";

/** The panel as the shell holds it, opened on `start` with `quote`. */
function Harness(props: {
  readonly client: FakeDaemon;
  readonly start?: string;
  readonly quote?: string;
}): ReactElement {
  const chat = useMainChat(leadFallback);
  const { openOn } = chat;
  useEffect(() => {
    if (props.start !== undefined) {
      openOn(props.start, props.quote);
    }
  }, [openOn, props.start, props.quote]);
  return (
    <MainChat
      client={props.client}
      connected
      canControl
      bots={BOTS}
      projects={[fx.project({ id: "p1", name: "The Hermes", lead_bot_id: "b1" })]}
      threads={decodeOwnerThreads(ownerThreadsJson()).threads}
      chat={chat}
      addToast={vi.fn<AddToast>()}
      onOpenBot={vi.fn<(botId: string) => void>()}
      now={HOME_NOW}
    />
  );
}

describe("MainChat", () => {
  it("opens with ⌘J on the bot in context and closes again", () => {
    render(<Harness client={daemon()} />);
    expect(screen.queryByRole("complementary", { name: "Chat" })).not.toBeInTheDocument();
    act(() => {
      fireEvent.keyDown(window, { key: "j", metaKey: true });
    });
    expect(screen.getByRole("complementary", { name: "Chat" })).toBeInTheDocument();
    expect(screen.getByRole("combobox", { name: "To" })).toHaveValue("b1");
    act(() => {
      fireEvent.keyDown(window, { key: "j", metaKey: true });
    });
    expect(screen.queryByRole("complementary", { name: "Chat" })).not.toBeInTheDocument();
  });

  it("lists a thread per bot and shows one, marking it read", async () => {
    const user = userEvent.setup();
    const client = daemon();
    render(<Harness client={client} start="b1" />);
    const threads = within(screen.getByRole("navigation", { name: "Threads" }));
    expect(threads.getByLabelText("asks you")).toBeInTheDocument();
    await user.click(threads.getByRole("button", { name: /Desktop Dev/ }));
    expect(await screen.findByText("Pushed 058a4f7.")).toBeInTheDocument();
    expect(
      screen.getByText("Should Resume now also restart the services?", {
        selector: ".mc-bubble-text",
      }),
    ).toBeInTheDocument();
    await waitFor(() => {
      expect(client.requests.map((r) => r.body)).toContainEqual({
        type: "owner_thread_read",
        bot_id: "b2",
        up_to_num: 12,
      });
    });
  });

  it("answers a report with it quoted, then sends to the bot picked", async () => {
    const user = userEvent.setup();
    const client = daemon();
    render(<Harness client={client} start="b2" quote="Should Resume restart services?" />);
    expect(screen.getByText("Replying to")).toBeInTheDocument();
    await user.type(screen.getByRole("textbox", { name: "Message" }), "Yes, restart them.");
    await user.keyboard("{Meta>}{Enter}{/Meta}");
    await waitFor(() => {
      expect(client.requests.map((r) => r.body)).toContainEqual({
        type: "send_user_message",
        to_bot_id: "b2",
        body: "> Should Resume restart services?\n\nYes, restart them.",
      });
    });
    expect(screen.queryByText("Replying to")).not.toBeInTheDocument();
    expect(screen.getByRole("textbox", { name: "Message" })).toHaveValue("");

    await user.selectOptions(screen.getByRole("combobox", { name: "To" }), "b1");
    await user.type(screen.getByRole("textbox", { name: "Message" }), "Ship it.");
    await user.click(screen.getByRole("button", { name: "Send ⌘↩" }));
    await waitFor(() => {
      expect(client.requests.map((r) => r.body)).toContainEqual({
        type: "send_user_message",
        to_bot_id: "b1",
        body: "Ship it.",
      });
    });
  });
});
