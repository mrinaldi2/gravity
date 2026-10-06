import type { Story } from "@ladle/react";
import type { ReactElement } from "react";
import type { ProjectTab } from "../../app/selection";
import { decodeOverview } from "../../protocol/home";
import { project } from "../../test/fixtures";
import { overviewJson } from "../../test/homeFixtures";
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
