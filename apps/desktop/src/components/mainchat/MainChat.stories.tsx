import type { Story } from "@ladle/react";
import { useEffect } from "react";
import type { ReactElement } from "react";
import { decodeOwnerThreads } from "../../protocol/home";
import { FakeDaemon } from "../../test/fakeDaemon";
import { bot, project } from "../../test/fixtures";
import { HOME_NOW, ownerThreadJson, ownerThreadsJson } from "../../test/homeFixtures";
import MainChat from "./MainChat";
import { useMainChat } from "./useMainChat";

const noop = (): void => {};
const lead = (): string => "b1";

function Panel(props: {
  readonly quote?: string;
  readonly fresh?: boolean;
  readonly unverified?: boolean;
}): ReactElement {
  const chat = useMainChat(lead);
  const { openOn, startNew } = chat;
  useEffect(() => {
    openOn("b2", props.quote);
    if (props.fresh === true) {
      startNew();
    }
  }, [openOn, startNew, props.quote, props.fresh]);
  const client = new FakeDaemon().onRequest("owner_thread_get", () => ({
    type: "owner_thread",
    req_id: "1",
    owner_thread: ownerThreadJson({ unverified: props.unverified === true }),
  }));
  client.capabilities = [...client.capabilities, "owner_threads"];
  return (
    <div className="app" style={{ height: 560, justifyContent: "flex-end" }}>
      <MainChat
        client={client}
        connected
        canControl
        bots={[bot({ id: "b1", name: "Team Lead" }), bot({ id: "b2", name: "Desktop Dev" })]}
        projects={[project({ id: "p1", name: "The Hermes", lead_bot_id: "b1" })]}
        threads={decodeOwnerThreads(ownerThreadsJson()).threads}
        chat={chat}
        addToast={noop}
        onOpenBot={noop}
        now={HOME_NOW}
      />
    </div>
  );
}

/** Threads per bot; Desktop Dev's open, its question last. */
export const Thread: Story = () => <Panel />;

/** Answering a report: it is quoted above the message. */
export const Replying: Story = () => <Panel quote="Should Resume now also restart the services?" />;

/** The owner's chat from a linked computer: from there, unverified (H-306). */
export const Unverified: Story = () => <Panel unverified />;

/** "＋ New message": nobody picked yet, the To picker waiting (H-157). */
export const NewMessage: Story = () => <Panel fresh />;
