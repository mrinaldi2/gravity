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

function Panel(props: { readonly quote?: string }): ReactElement {
  const chat = useMainChat(lead);
  const { openOn } = chat;
  useEffect(() => {
    openOn("b2", props.quote);
  }, [openOn, props.quote]);
  const client = new FakeDaemon().onRequest("owner_thread_get", () => ({
    type: "owner_thread",
    req_id: "1",
    owner_thread: ownerThreadJson(),
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
