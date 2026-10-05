import type { Story } from "@ladle/react";
import type { ReactElement } from "react";
import type { Release } from "../../protocol/releases";
import { FakeDaemon } from "../../test/fakeDaemon";
import { bot, project } from "../../test/fixtures";
import {
  CANCELLED,
  MAC_SHA,
  RELEASE_TITLES,
  WIN_SHA,
  deployment,
  release,
} from "../../test/releaseFixtures";
import ProjectWindow from "../project/ProjectWindow";
import ReleaseReview from "./ReleaseReview";
import ReleasesView from "./ReleasesView";
import { useReleaseActions } from "./useReleases";

const noop = (): void => undefined;

function Review({ value }: { readonly value: Release }): ReactElement {
  const actions = useReleaseActions(new FakeDaemon(), noop, noop);
  return (
    <div className="main" style={{ height: 640, display: "flex" }}>
      <ReleaseReview
        release={value}
        titles={RELEASE_TITLES}
        botName={(id) => (id === "ops" ? "DevOps" : id === "tester" ? "Tester" : "Tester Win")}
        actions={actions}
        canControl
        now={() => Date.parse("2026-10-05T12:00:00Z")}
      />
    </div>
  );
}

export const Ready: Story = () => <Review value={release()} />;

export const FailedTest: Story = () => (
  <Review
    value={release({
      tests: [
        { machine: "mac", tester: "tester", build_sha256: MAC_SHA, result: "pass" },
        { machine: "win-pc", tester: "tester-win", build_sha256: WIN_SHA, result: "fail" },
      ],
    })}
  />
);

export const CannotRule: Story = () => (
  <Review value={release({ can_rule: false, rule_on: "Mac" })} />
);

export const Held: Story = () => (
  <Review
    value={release({
      status: "held",
      held_note: "after the trip",
      remind_at: "2026-10-20T09:00:00Z",
    })}
  />
);

export const Repackaging: Story = () => (
  <Review
    value={release({
      status: "repackaging",
      items: [
        { item_id: "H-017", verdict: "ship", owner_note: null },
        { item_id: "H-020", verdict: "rework", owner_note: "font falls back on win-pc" },
      ],
    })}
  />
);

export const RollingOut: Story = () => (
  <Review
    value={release({
      status: "deploying",
      deployments: [
        deployment({ machine: "mac", result: "ok", smoke: "pass", at: "2026-10-05T16:03:00Z" }),
        deployment({ machine: "win-pc", executor: "tester-win" }),
      ],
    })}
  />
);

export const Paused: Story = () => (
  <Review
    value={release({
      status: "paused",
      paused_reason: "crash on launch on win-pc",
      deployments: [deployment({ machine: "mac", result: "ok" })],
    })}
  />
);

/** A successor DevOps cancelled shows on the package it would have replaced. */
export const SuccessorCancelled: Story = () => (
  <Review
    value={release({
      status: "repackaging",
      items: [
        { item_id: "H-017", verdict: "ship", owner_note: null },
        { item_id: "H-020", verdict: "rework", owner_note: "font falls back on win-pc" },
      ],
      events: [CANCELLED],
    })}
  />
);

/** The Releases tab in the project window, with a current package and history. */
export const Tab: Story = () => {
  const acme = project({ id: "p1", name: "The Hermes" });
  const client = new FakeDaemon().onRequest("list_releases", () => ({
    type: "releases",
    req_id: "1",
    releases: [
      release(),
      release({ id: "rel-0", display_version: "0.15.2", status: "deployed", decision_id: null }),
      release({ id: "rel-x", display_version: "0.15.1", status: "rejected", decision_id: null }),
    ],
  }));
  return (
    <div className="main" style={{ height: 640 }}>
      <ProjectWindow project={acme} botCount={3} tab="releases" onSelectTab={noop}>
        <ReleasesView
          client={client}
          project={acme}
          bots={[bot({ id: "ops", name: "DevOps", project_id: "p1" })]}
          connected
          canControl
          addToast={noop}
        />
      </ProjectWindow>
    </div>
  );
};
