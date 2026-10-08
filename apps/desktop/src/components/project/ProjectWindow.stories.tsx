import type { Story } from "@ladle/react";
import { useState } from "react";
import type { ReactElement } from "react";
import type { ProjectTab } from "../../app/selection";
import { decodeOverview, decodeOwnerThreads } from "../../protocol/home";
import { bot, project } from "../../test/fixtures";
import { HOME_NOW, overviewJson, ownerThreadsJson } from "../../test/homeFixtures";
import TeamView from "../team/TeamView";
import ProjectWindow from "./ProjectWindow";

const noop = (_tab: ProjectTab): void => {};
const row = decodeOverview(overviewJson()).rows.find((r) => r.projectId === "p1") ?? null;

function Window({ tab }: { readonly tab: ProjectTab }): ReactElement {
  return (
    <div className="main" style={{ height: 300 }}>
      <ProjectWindow
        project={project({ name: "The Hermes" })}
        botCount={8}
        tab={tab}
        row={row}
        onHome={() => {}}
        onSelectTab={noop}
      >
        <p style={{ padding: 16 }}>The {tab} view.</p>
      </ProjectWindow>
    </div>
  );
}

/** The header with what needs you and the release, and the five tabs plus More. */
export const Overview: Story = () => <Window tab="overview" />;
/** A tab under More: the More button names it. */
export const Settings: Story = () => <Window tab="settings" />;

const TEAM = [
  bot({ id: "b1", name: "Team Lead", state: "working" }),
  bot({ id: "b2", name: "Desktop Dev", state: "ready" }),
  bot({ id: "b3", name: "iOS QA", state: "ready" }),
];

/**
 * The Team tab live: tabs switch, and each card opens its own bot. The
 * click test in `tests/visual/project-clicks.spec.ts` drives it (H-189).
 */
export const TeamClicks: Story = () => {
  const [tab, setTab] = useState<ProjectTab>("team");
  const [opened, setOpened] = useState("none");
  return (
    <div className="main" style={{ height: 600 }}>
      <ProjectWindow
        project={project({ name: "The Hermes" })}
        botCount={TEAM.length}
        tab={tab}
        row={row}
        onHome={() => {}}
        onSelectTab={setTab}
      >
        <p style={{ padding: "4px 16px" }} data-testid="opened">
          Opened: {opened}
        </p>
        {tab === "team" ? (
          <TeamView
            bots={TEAM}
            row={row}
            threads={decodeOwnerThreads(ownerThreadsJson()).threads}
            unread={{}}
            failed={{}}
            activity={{}}
            leadBotId="b1"
            now={HOME_NOW}
            canControl
            onOpenBot={setOpened}
            onCreateBot={() => {}}
            onDeleteBot={() => {}}
          />
        ) : (
          <p style={{ padding: 16 }}>The {tab} view.</p>
        )}
      </ProjectWindow>
    </div>
  );
};
