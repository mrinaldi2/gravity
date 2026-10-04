import { useCallback } from "react";
import type { ReactElement } from "react";
import type { ProjectTab } from "../app/selection";
import type { DaemonState } from "../app/useDaemonState";
import type { AddToast } from "../app/useToasts";
import heroArt from "../assets/empty/hero.png";
import quietArt from "../assets/empty/quiet.png";
import type { DaemonApi } from "../protocol/api";
import { connectionStatusLabel } from "../protocol/connection";
import type { Project, ProjectRepo } from "../protocol/entities";
import BotView from "./BotView";
import ControlCenterView from "./control/ControlCenterView";
import type { Permissions } from "./permissions/usePermissions";
import ConversationsView from "./conversations/ConversationsView";
import ComingSoon, { isUpcomingTab } from "./project/ComingSoon";
import ProjectWindow from "./project/ProjectWindow";
import ProjectView from "./ProjectView";
import NewProjectForm from "./sidebar/NewProjectForm";

interface MainPaneProps {
  readonly client: DaemonApi;
  readonly daemon: DaemonState;
  readonly addToast: AddToast;
  readonly onCreateProject: (name: string) => Promise<void>;
  readonly onRenameProject: (projectId: string, name: string) => Promise<void>;
  readonly onSetProjectLead: (projectId: string, botId: string | null) => Promise<void>;
  readonly onSetProjectRepo: (projectId: string, repo: ProjectRepo | null) => Promise<void>;
  readonly onDeleteProject: (projectId: string) => Promise<void>;
  /** Every bot's permission requests, answered in Decisions. */
  readonly permissions?: Permissions;
  readonly onOpenBot?: (botId: string) => void;
}

interface EmptyStateProps {
  readonly daemon: DaemonState;
  readonly onCreateProject: (name: string) => Promise<void>;
}

/** The first-launch pane: nothing exists yet, so it carries the onboarding. */
function WelcomeState({ daemon, onCreateProject }: EmptyStateProps): ReactElement {
  return (
    // No header here, so the whole pane doubles as the window drag strip.
    <div className="empty-pane" data-tauri-drag-region="deep">
      <div className="empty-state">
        <img className="empty-art-hero" src={heroArt} alt="" draggable={false} />
        <h1>Welcome to The Hermes</h1>
        <p>Projects hold your bots. Create one to get started.</p>
        {daemon.canControl ? <NewProjectForm onCreate={onCreateProject} /> : null}
      </div>
    </div>
  );
}

function EmptyState(props: EmptyStateProps): ReactElement {
  const { daemon } = props;
  const { endpoint, status, connected } = daemon;
  if (connected && daemon.projects.length === 0) {
    return <WelcomeState {...props} />;
  }
  return (
    // No header here, so the whole pane doubles as the window drag strip.
    <div className="empty-pane" data-tauri-drag-region="deep">
      <div className="empty-state">
        <img className="empty-art-quiet" src={quietArt} alt="" draggable={false} />
        <h1>The Hermes</h1>
        <p>Select a bot from the sidebar, or create one to get started.</p>
        {connected ? null : (
          <p className={`conn-hint conn-${status}`}>
            {`Hermes service: ${connectionStatusLabel(status)} (${endpoint.host}:${endpoint.port})`}
          </p>
        )}
      </div>
    </div>
  );
}

interface ProjectPaneProps extends MainPaneProps {
  readonly project: Project;
  readonly tab: ProjectTab;
}

/** A project window showing `tab`. */
function ProjectPane(props: ProjectPaneProps): ReactElement {
  const { client, daemon, project, tab } = props;
  const { connected, canControl, select } = daemon;
  const bots = daemon.bots.filter((item) => item.project_id === project.id);
  const onSelectTab = useCallback(
    (next: ProjectTab): void => {
      select({ kind: "project", projectId: project.id, tab: next });
    },
    [select, project.id],
  );

  let content: ReactElement;
  if (isUpcomingTab(tab)) {
    content = <ComingSoon tab={tab} />;
  } else if (tab === "conversations") {
    content = client.capabilities.includes("agent_conversations") ? (
      <ConversationsView client={client} project={project} bots={bots} connected={connected} />
    ) : (
      <div className="empty-pane">
        <div className="empty-state">
          <p>Conversations need a newer Hermes service.</p>
        </div>
      </div>
    );
  } else {
    content = (
      <ProjectView
        client={client}
        project={project}
        bots={bots}
        connected={connected}
        canControl={canControl}
        onRename={props.onRenameProject}
        onSetLead={props.onSetProjectLead}
        onSetRepo={props.onSetProjectRepo}
        onDelete={props.onDeleteProject}
        onToast={props.addToast}
      />
    );
  }

  return (
    <ProjectWindow
      key={project.id}
      project={project}
      botCount={bots.length}
      tab={tab}
      onSelectTab={onSelectTab}
    >
      {content}
    </ProjectWindow>
  );
}

/** Renders whichever view the current selection points at. */
export default function MainPane(props: MainPaneProps): ReactElement {
  const { client, daemon, addToast } = props;
  const { selection, bots, connected, canControl } = daemon;

  if (selection.kind === "control") {
    return (
      <ControlCenterView
        client={client}
        projects={daemon.projects}
        bots={bots}
        connected={connected}
        canControl={canControl}
        decisionId={selection.decisionId}
        onToast={addToast}
        permissions={props.permissions}
        onOpenBot={props.onOpenBot}
      />
    );
  }

  if (selection.kind === "project") {
    const project = daemon.projects.find((item) => item.id === selection.projectId);
    return project === undefined ? (
      <EmptyState daemon={daemon} onCreateProject={props.onCreateProject} />
    ) : (
      <ProjectPane {...props} project={project} tab={selection.tab ?? "dashboard"} />
    );
  }

  if (selection.kind === "bot") {
    const bot = bots.find((item) => item.id === selection.botId);
    return bot === undefined ? (
      <EmptyState daemon={daemon} onCreateProject={props.onCreateProject} />
    ) : (
      <BotView
        key={bot.id}
        client={client}
        bot={bot}
        bots={bots}
        connected={connected}
        canControl={canControl}
        onBotUpdated={daemon.applyBotUpdate}
        onRoutinesChanged={daemon.updateBotRoutines}
        onToast={addToast}
        onOpenDecision={(decisionId) => {
          daemon.select({ kind: "control", decisionId });
        }}
      />
    );
  }

  return <EmptyState daemon={daemon} onCreateProject={props.onCreateProject} />;
}
