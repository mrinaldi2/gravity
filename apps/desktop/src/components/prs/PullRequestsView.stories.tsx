import type { Story } from "@ladle/react";
import { useEffect, useRef, useState } from "react";
import type { ReactElement, RefObject } from "react";
import { CARD_BOTS, CARD_PROJECT } from "../../test/cardFixtures";
import { mergedPr, PR_NOW, prDaemon, prList, waitingPr } from "../../test/prFixtures";
import { CardLinksProvider } from "../cards/CardLinks";
import PullRequestsView from "./PullRequestsView";

const noop = (): void => {};

/** Clicks the button whose text starts with `label` once it shows. */
function useClickOnce(box: RefObject<HTMLElement | null>, label: string | undefined): void {
  useEffect(() => {
    if (label === undefined) {
      return undefined;
    }
    const timer = setInterval(() => {
      const button = [...(box.current?.querySelectorAll<HTMLButtonElement>("button") ?? [])].find(
        (b) => b.textContent?.startsWith(label) === true,
      );
      if (button !== undefined) {
        button.click();
        clearInterval(timer);
      }
    }, 10);
    return () => clearInterval(timer);
  }, [box, label]);
}

function Frame(props: {
  readonly light?: boolean;
  readonly number?: number;
  /** A filter or section to open once it shows: "Open", "Files". */
  readonly click?: string;
}): ReactElement {
  // The list has #42 ready for you; its detail shows it still waiting for CE.
  const [client] = useState(() => prDaemon(prList(), [waitingPr()]));
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
        />
      </div>
    </CardLinksProvider>
  );
}

/** The list opens on "Waiting for you" whenever anything waits for you. */
export const List: Story = () => <Frame />;

/** Open PRs: behind main, conflicts, and who each waits for. */
export const ListOpen: Story = () => <Frame click="Open" />;

export const ListLight: Story = () => <Frame light click="Open" />;

/** #42's overview: the "Waiting for" line, verdicts at their commits, findings, checks at the head. */
export const Detail: Story = () => <Frame number={42} />;

export const DetailLight: Story = () => <Frame light number={42} />;

/** #42's files and unified diff. */
export const DetailFiles: Story = () => <Frame number={42} click="Files" />;

/** #41 merged: "✓ Merged into main · 09:12" and its cleanup. */
export const DetailMerged: Story = () => <Frame number={mergedPr().number} />;
