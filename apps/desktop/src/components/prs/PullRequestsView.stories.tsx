import type { Story } from "@ladle/react";
import type { ReactElement } from "react";
import { mergedPr, prDaemon, prList, waitingPr } from "../../test/prFixtures";
import { PrStoryFrame } from "../../test/prStoryFrame";

// The list has #42 ready for you; its detail shows it still waiting for CE.
const daemon = () => prDaemon(prList(), [waitingPr()]);

function Frame(props: {
  readonly light?: boolean;
  readonly number?: number;
  /** A filter or section to open once it shows: "Open", "Files". */
  readonly click?: string;
}): ReactElement {
  return <PrStoryFrame client={daemon} {...props} />;
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
