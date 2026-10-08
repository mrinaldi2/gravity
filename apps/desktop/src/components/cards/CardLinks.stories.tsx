import type { Story } from "@ladle/react";
import { useEffect, useRef, useState } from "react";
import type { ReactElement, ReactNode } from "react";
import { CARD_BOTS, CARD_PROJECT, cardsDaemon } from "../../test/cardFixtures";
import ChatMarkdown from "../chat/ChatMarkdown";
import { CardLinksProvider } from "./CardLinks";
import LinkedText from "./LinkedText";

const noop = (): void => {};

function Frame(props: { readonly children: ReactNode }): ReactElement {
  const [client] = useState(cardsDaemon);
  return (
    <CardLinksProvider
      client={client}
      projects={[CARD_PROJECT]}
      bots={CARD_BOTS}
      currentProjectId="p1"
      onOpen={noop}
    >
      <div style={{ padding: 24, maxWidth: 560, minHeight: 240 }}>{props.children}</div>
    </CardLinksProvider>
  );
}

/** A link with its preview open, as keyboard focus shows it. */
function Focused(props: { readonly text: string }): ReactElement {
  const box = useRef<HTMLParagraphElement>(null);
  useEffect(() => {
    box.current?.querySelector<HTMLElement>(".card-link")?.focus();
  }, []);
  return (
    <Frame>
      <p ref={box}>
        <LinkedText text={props.text} />
      </p>
    </Frame>
  );
}

/** Ids in a bot's Markdown and in plain text; code, branches and UTF-8 stay text. */
export const LinkedIds: Story = () => (
  <Frame>
    <ChatMarkdown>
      {
        "Fixed **H-293** on branch H-293-clicks; see `H-292` and UTF-8 handling.\n\n```\ngit log H-291\n```"
      }
    </ChatMarkdown>
    <p className="toast-body">
      <LinkedText text="H-293 moved to Review. H-999 was mistyped." />
    </p>
  </Frame>
);

/** The preview: id, type and priority, title, column and assignee. */
export const Preview: Story = () => <Focused text="Fixed in H-293, as the owner asked." />;

/** A card that isn't on the board says so, and the link is dimmed. */
export const NotOnTheBoard: Story = () => <Focused text="See H-999 for the old plan." />;

/** A card kept on a computer that's offline, with the title last seen. */
export const KeptOffline: Story = () => <Focused text="H-500 waits on win-pc." />;
