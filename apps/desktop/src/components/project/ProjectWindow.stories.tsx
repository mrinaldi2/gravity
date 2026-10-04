import type { Story } from "@ladle/react";
import type { ReactElement } from "react";
import type { ProjectTab } from "../../app/selection";
import { project } from "../../test/fixtures";
import ComingSoon from "./ComingSoon";
import type { UpcomingTab } from "./ComingSoon";
import ProjectWindow from "./ProjectWindow";

const noop = (_tab: ProjectTab): void => {};

function Window({ tab }: { readonly tab: UpcomingTab }): ReactElement {
  return (
    <div className="main" style={{ height: 600 }}>
      <ProjectWindow
        project={project({ name: "The Hermes" })}
        botCount={6}
        tab={tab}
        onSelectTab={noop}
      >
        <ComingSoon tab={tab} />
      </ProjectWindow>
    </div>
  );
}

export const Dashboard: Story = () => <Window tab="dashboard" />;
export const Board: Story = () => <Window tab="board" />;
export const Releases: Story = () => <Window tab="releases" />;
export const Meetings: Story = () => <Window tab="meetings" />;
