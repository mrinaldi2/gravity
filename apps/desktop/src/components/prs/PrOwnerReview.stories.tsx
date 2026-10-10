import type { Story } from "@ladle/react";
import { useState } from "react";
import type { ReactElement } from "react";
import type { PullRequest } from "../../protocol/gen/hermes/pr/v1/pr_pb";
import { mergingPr, ownerDaemon, recheckPr } from "../../test/ownerReviewFixtures";
import { readyForYouPr } from "../../test/prFixtures";
import { PrStoryFrame } from "../../test/prStoryFrame";
import OwnerReviewSettings from "./OwnerReviewSettings";

function Frame(props: {
  readonly pr: PullRequest;
  readonly light?: boolean;
  readonly recheck?: boolean;
  /** A section or button to press once it shows: "Files", "Review…". */
  readonly click?: string;
}): ReactElement {
  const { pr, ...rest } = props;
  return <PrStoryFrame client={() => ownerDaemon([pr])} number={pr.number} {...rest} />;
}

/** #42 waits only for you: Review… on your row, and the Owner review line. */
export const Ready: Story = () => <Frame pr={readyForYouPr()} />;

/** Your review: Approve says what it triggers; Ask for changes needs a note. */
export const ReviewDialog: Story = () => <Frame pr={readyForYouPr()} click="Review…" />;

export const ReviewDialogLight: Story = () => <Frame pr={readyForYouPr()} click="Review…" light />;

/** After your Approve: the 10 s Undo, nothing pushed yet. */
export const UndoWindow: Story = () => <Frame pr={mergingPr(7)} />;

export const UndoWindowLight: Story = () => <Frame pr={mergingPr(7)} light />;

/** The window is over and no DevOps has taken the merge yet. */
export const WaitingForDevops: Story = () => <Frame pr={mergingPr(-30)} />;

/** Line comments under their lines: a thread with a reply, a resolved one, an outdated one. */
export const FilesComments: Story = () => <Frame pr={readyForYouPr()} click="Files" />;

export const FilesCommentsLight: Story = () => <Frame pr={readyForYouPr()} click="Files" light />;

/** A Re-check from Needs you: only what changed since you approved. */
export const Recheck: Story = () => <Frame pr={recheckPr()} recheck />;

/** A service whose setting is Some areas: security and docs. */
function areasDaemon() {
  return ownerDaemon().onRequest("review_settings_get", () => ({
    type: "review_settings",
    req_id: "r",
    review_settings: {
      project_id: "p1",
      owner_review: "areas",
      owner_review_areas: ["security", "docs"],
      areas: ["security", "releases", "docs", "desktop", "ios", "service"],
    },
  }));
}

function Settings(props: { readonly light?: boolean; readonly areas?: boolean }): ReactElement {
  const [client] = useState(() => (props.areas ? areasDaemon() : ownerDaemon()));
  return (
    <div
      className={`project-view ${props.light ? "theme-light" : ""}`}
      style={{ minHeight: "100vh", background: "var(--bg)", padding: 20 }}
    >
      <OwnerReviewSettings client={client} projectId="p1" connected />
    </div>
  );
}

/** Settings › Owner review, on its default. */
export const OwnerReviewSetting: Story = () => <Settings />;

/** Some areas, picked from reviewers.toml's areas. */
export const OwnerReviewSettingLight: Story = () => <Settings light areas />;
