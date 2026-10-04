import type { Story } from "@ladle/react";
import type { Selection } from "../../app/selection";
import { bot, project } from "../../test/fixtures";
import ProjectSection from "./ProjectSection";

const noop = (): void => {};
const resolved = (): Promise<void> => Promise.resolve();

const BOTS = [
  bot({ id: "b1", name: "Team Lead", description: "Runs the team", avatar: "icon:orbit" }),
  bot({ id: "b2", name: "Desktop Dev", description: "Builds the desktop app", state: "working" }),
] as const;

function Section({ selection }: { readonly selection: Selection }): ReturnType<Story> {
  return (
    <div style={{ width: 268 }}>
      <ProjectSection
        project={project({ name: "The Hermes" })}
        bots={BOTS}
        unreadBots={{}}
        failedByBot={new Map()}
        nextRun={{}}
        pinnedBotIds={[]}
        activityByBot={{}}
        selection={selection}
        canControl
        onSelect={noop}
        onCreateBot={noop}
        onDeleteBot={resolved}
        onDeleteProject={resolved}
        onTogglePin={noop}
      />
    </div>
  );
}

export const Expanded: Story = () => <Section selection={{ kind: "none" }} />;

export const ProjectOpen: Story = () => (
  <Section selection={{ kind: "project", projectId: "p1", tab: "dashboard" }} />
);
