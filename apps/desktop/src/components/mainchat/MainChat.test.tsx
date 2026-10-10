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

function daemon(unverified = false): FakeDaemon {
  const fake = new FakeDaemon()
    .onRequest("owner_thread_get", () => ({
      type: "owner_thread",
      req_id: "1",
      owner_thread: ownerThreadJson({ unverified }),
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
  readonly onOpenBot?: (botId: string) => void;
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
      onOpenBot={props.onOpenBot ?? vi.fn<(botId: string) => void>()}
      now={HOME_NOW}
    />
  );
}

describe("MainChat", () => {
  it("opens the bot's page from a visible button in the thread's header (H-192)", async () => {
    const onOpenBot = vi.fn<(botId: string) => void>();
    render(<Harness client={daemon()} start="b2" onOpenBot={onOpenBot} />);
    const open = screen.getByRole("button", { name: "Open Desktop Dev's page" });
    expect(open).toHaveTextContent("Open Desktop Dev");
    await userEvent.click(open);
    expect(onOpenBot).toHaveBeenCalledWith("b2");
  });

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

  it("shows the owner's chat from a linked computer as from there, unverified (H-306)", async () => {
    render(<Harness client={daemon(true)} start="b2" />);
    const text = await screen.findByText("Yes, restart them too.");
    const bubble = text.closest(".mc-bubble");
    expect(bubble).toHaveClass("mc-bubble-unverified");
    expect(bubble).not.toHaveClass("mc-bubble-me");
    expect(bubble).toHaveTextContent("From MacBook, unverified");
    expect(bubble).not.toHaveTextContent("You");
    // It answers nothing: the question before it is still open.
    expect(screen.getByText(/^Asks you/)).toBeInTheDocument();
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

  it("starts a new message from the thread list, in the To picker", async () => {
    const user = userEvent.setup();
    render(<Harness client={daemon()} start="b2" quote="Should Resume restart?" />);
    const threads = screen.getByRole("navigation", { name: "Threads" });
    await user.click(
      within(threads).getByRole("button", { name: "New message: pick who to write to" }),
    );
    const to = screen.getByRole("combobox", { name: "To" });
    expect(to).toHaveFocus();
    expect(to).toHaveValue("");
    expect(screen.getByText("Pick who to write to.")).toBeInTheDocument();
    expect(screen.queryByText("Replying to")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Send/ })).toBeDisabled();
    await user.selectOptions(to, "b1");
    expect(to).toHaveValue("b1");
  });
});
