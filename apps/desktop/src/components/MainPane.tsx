import type { ReactElement } from "react";
import type { DaemonState } from "../app/useDaemonState";
import heroArt from "../assets/empty/hero.png";
import quietArt from "../assets/empty/quiet.png";
import { connectionStatusLabel } from "../protocol/connection";
import BotView from "./BotView";
import ControlCenterView from "./control/ControlCenterView";
import type { Permissions } from "./permissions/usePermissions";
import ProjectsHome from "./home/ProjectsHome";
import ProjectPane from "./project/ProjectPane";
import type { ProjectActions } from "./project/ProjectPane";
import NewProjectForm from "./sidebar/NewProjectForm";

interface MainPaneProps extends ProjectActions {
  readonly onCreateProject: (name: string) => Promise<void>;
  /** Every bot's permission requests, answered in Decisions. */
  readonly permissions?: Permissions;
  readonly onOpenBot?: (botId: string) => void;
  /** Answers a bot quoting one of its reports; else Reply opens the bot. */
  readonly onReply?: (botId: string, quote: string) => void;
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
        <p>{connected ? "This is no longer here." : "Waiting for the Hermes service."}</p>
        {connected ? null : (
          <p className={`conn-hint conn-${status}`}>
            {`Hermes service: ${connectionStatusLabel(status)} (${endpoint.host}:${endpoint.port})`}
          </p>
        )}
      </div>
    </div>
  );
}

/** Renders whichever view the current selection points at. */
export default function MainPane(props: MainPaneProps): ReactElement {
  const { client, daemon, addToast } = props;
  const { selection, bots, connected, canControl } = daemon;
  const reply =
    props.onReply ??
    ((botId: string): void => {
      daemon.select({ kind: "bot", botId });
    });

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
      <ProjectPane {...props} project={project} tab={selection.tab ?? "overview"} onReply={reply} />
    );
  }

  if (selection.kind === "bot") {
    const bot = bots.find((item) => item.id === selection.botId);
    return bot === undefined ? (
      <EmptyState daemon={daemon} onCreateProject={props.onCreateProject} />
    ) : (
      <BotView
        key={`${bot.id}:${selection.tab ?? ""}`}
        client={client}
        bot={bot}
        bots={bots}
        initialTab={selection.tab}
        connected={connected}
        canControl={canControl}
        onBotUpdated={daemon.applyBotUpdate}
        onRoutinesChanged={daemon.updateBotRoutines}
        onToast={addToast}
        onOpenDecision={(decisionId) => {
          daemon.select({ kind: "control", decisionId });
        }}
        onReply={props.onReply}
        onBack={() => {
          daemon.select({ kind: "project", projectId: bot.project_id, tab: "team" });
        }}
      />
    );
  }

  if (connected && daemon.projects.length > 0) {
    return (
      <ProjectsHome
        client={client}
        projects={daemon.projects}
        bots={bots}
        connected={connected}
        canControl={canControl}
        addToast={addToast}
        onOpenProject={(projectId) => {
          daemon.select({ kind: "project", projectId });
        }}
        onCreateProject={props.onCreateProject}
      />
    );
  }
  return <EmptyState daemon={daemon} onCreateProject={props.onCreateProject} />;
}
