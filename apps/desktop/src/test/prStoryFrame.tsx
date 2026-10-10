// The Pull requests tab as stories show it: card links, a fixed now, light or
// dark, and one click in (a filter, a section, a dialog).

import { useRef, useState } from "react";
import type { ReactElement } from "react";
import { CardLinksProvider } from "../components/cards/CardLinks";
import PullRequestsView from "../components/prs/PullRequestsView";
import { CARD_BOTS, CARD_PROJECT } from "./cardFixtures";
import type { FakeDaemon } from "./fakeDaemon";
import { PR_NOW } from "./prFixtures";
import { useClickOnce } from "./storyClick";

const noop = (): void => {};

export function PrStoryFrame(props: {
  /** Made once, on the first render. */
  readonly client: () => FakeDaemon;
  readonly light?: boolean;
  readonly number?: number;
  readonly recheck?: boolean;
  /** A filter, section or button to press once it shows: "Open", "Files", "Review…". */
  readonly click?: string;
}): ReactElement {
  const [client] = useState(props.client);
  const box = useRef<HTMLDivElement>(null);
  useClickOnce(box, props.click);
  return (
    <CardLinksProvider
      client={client}
      projects={[CARD_PROJECT]}
      bots={CARD_BOTS}
      currentProjectId="p1"
      onOpen={noop}
    >
      <div
        ref={box}
        className={props.light ? "theme-light" : undefined}
        style={{ height: "100vh" }}
      >
        <PullRequestsView
          client={client}
          project={CARD_PROJECT}
          connected
          now={PR_NOW}
          initialNumber={props.number}
          recheck={props.recheck}
          onOpenSettings={noop}
          live={false}
        />
      </div>
    </CardLinksProvider>
  );
}
