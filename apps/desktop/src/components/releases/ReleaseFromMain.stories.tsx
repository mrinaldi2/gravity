import type { Story } from "@ladle/react";
import type { ReactElement } from "react";
import type { Release } from "../../protocol/releases";
import { FakeDaemon } from "../../test/fakeDaemon";
import { MAIN_RECORDS, MAIN_TITLES, mainRelease } from "../../test/releaseMainFixtures";
import { PrLeaveOutDialog } from "./FromMain";
import ReleaseReview from "./ReleaseReview";
import { useReleaseActions } from "./useReleases";

const noop = (): void => undefined;

/** A release cut from main (H-278), in dark or in UX-051's light tokens. */
function Frame(props: {
  readonly value: Release;
  readonly light?: boolean;
  readonly children?: ReactElement;
}): ReactElement {
  const actions = useReleaseActions(new FakeDaemon(), noop, noop);
  return (
    <div className={props.light ? "theme-light" : undefined} style={{ minHeight: "100vh" }}>
      <div className="main" style={{ height: 640, display: "flex" }}>
        <ReleaseReview
          release={props.value}
          titles={MAIN_TITLES}
          botName={(id) => (id === "ops" ? "DevOps" : "Tester")}
          actions={actions}
          canControl
          now={() => Date.parse("2026-10-05T12:00:00Z")}
          previous="0.17.5"
          records={MAIN_RECORDS}
          onOpenPr={noop}
        />
      </div>
      {props.children ?? <></>}
    </div>
  );
}

export const FromMain: Story = () => <Frame value={mainRelease()} />;

export const FromMainLight: Story = () => <Frame value={mainRelease()} light />;

/** Leaving out an earlier PR: the undo pull request through the queue. */
function LeaveOut({ light }: { readonly light?: boolean }): ReactElement {
  const value = mainRelease({ also_included: [] });
  const pr = value.prs?.[0];
  return (
    <Frame value={value} light={light}>
      {pr ? (
        <PrLeaveOutDialog
          release={value}
          pr={pr}
          titles={MAIN_TITLES}
          onConfirm={noop}
          onCancel={noop}
        />
      ) : (
        <></>
      )}
    </Frame>
  );
}

export const LeaveOutUndo: Story = () => <LeaveOut />;

export const LeaveOutUndoLight: Story = () => <LeaveOut light />;
