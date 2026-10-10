// A project's window (UX-024, H-133 U2): Overview (Needs you, From the team
// and the dashboard), Board, Pull requests (when the service serves them,
// H-276), Team, Releases and Meetings, with Conversations
// and Settings under More. The project's overview row and the owner's threads
// are read once here and shared by the header, Overview and Team.

import { useCallback } from "react";
import type { ReactElement } from "react";
import type { ProjectTab, Selection } from "../../app/selection";
import type { DaemonState } from "../../app/useDaemonState";
import type { AddToast } from "../../app/useToasts";
import type { DaemonApi } from "../../protocol/api";
import type { Bot, Project, ProjectRepo } from "../../protocol/entities";
import type { OwnerThread, ProjectRow } from "../../protocol/gen/hermes/home/v1/home_pb";
import BoardView from "../board/BoardView";
import { useNow } from "../control/useNow";
import ConversationsView from "../conversations/ConversationsView";
import DashboardView from "../dashboard/DashboardView";
import FromTheTeam from "../dashboard/FromTheTeam";
import { useOwnerThreads } from "../home/useOwnerThreads";
import { useProjectRow } from "../home/useProjectsOverview";
import MeetingsView from "../meetings/MeetingsView";
import { useMeeting } from "../meetings/useMeetings";
import type { MeetingSummary } from "../../protocol/meetings";
import ProjectView from "../ProjectView";
import { PULL_REQUESTS } from "../../protocol/prs";
import PullRequestsView from "../prs/PullRequestsView";
import ReleasesView from "../releases/ReleasesView";
import TeamView from "../team/TeamView";
import ProjectWindow from "./ProjectWindow";

/** What the owner can change about a project, from its window. */
export interface ProjectActions {
  readonly client: DaemonApi;
  readonly daemon: DaemonState;
  readonly addToast: AddToast;
  readonly onRenameProject: (projectId: string, name: string) => Promise<void>;
  readonly onSetProjectLead: (projectId: string, botId: string | null) => Promise<void>;
  readonly onSetProjectRepo: (projectId: string, repo: ProjectRepo | null) => Promise<void>;
  readonly onDeleteProject: (projectId: string) => Promise<void>;
  readonly onCreateBot: (projectId: string) => Promise<void>;
  readonly onDeleteBot: (botId: string) => Promise<void>;
}

interface ProjectPaneProps extends ProjectActions {
  readonly project: Project;
  readonly tab: ProjectTab;
  /** Answers a bot, quoting a report of its own. */
  readonly onReply: (botId: string, quote: string) => void;
}

/** Failed deliveries per bot, for the Team cards' badge. */
function failedByBot(deliveries: readonly { readonly bot_id: string }[]): Record<string, number> {
  const counts: Record<string, number> = {};
  for (const delivery of deliveries) {
    counts[delivery.bot_id] = (counts[delivery.bot_id] ?? 0) + 1;
  }
  return counts;
}

function NeedsNewer({ what }: { readonly what: string }): ReactElement {
  return (
    <div className="empty-pane">
      <div className="empty-state">
        <p>{what} need a newer Hermes service.</p>
      </div>
    </div>
  );
}

/** What every tab's view draws on, read once by the window. */
interface TabContext extends ProjectPaneProps {
  readonly bots: readonly Bot[];
  readonly row: ProjectRow | null;
  readonly threads: readonly OwnerThread[];
  readonly summaryMeeting: MeetingSummary | null;
  readonly now: number;
  readonly onSelectTab: (tab: ProjectTab) => void;
  /** Opens one PR, or its delta since your approval (Needs you, H-277). */
  readonly onOpenPr: (pr: number, recheck: boolean) => void;
}

/** The Overview: the dashboard with From the team after Needs you. */
function Overview({ ctx }: { readonly ctx: TabContext }): ReactElement {
  const { client, daemon, project } = ctx;
  const select = daemon.select;
  return (
    <DashboardView
      client={client}
      project={project}
      bots={ctx.bots}
      connected={daemon.connected}
      canControl={daemon.canControl}
      addToast={ctx.addToast}
      onOpenTab={ctx.onSelectTab}
      onOpenPr={client.capabilities.includes(PULL_REQUESTS) ? ctx.onOpenPr : undefined}
      onOpenBot={(botId) => select({ kind: "bot", botId })}
      onOpenDecision={(decisionId) => select({ kind: "control", decisionId })}
      onReply={ctx.onReply}
      onOpenNeedsYou={() => select({ kind: "control" })}
      fromTheTeam={
        <FromTheTeam
          row={ctx.row}
          threads={ctx.threads}
          bots={ctx.bots}
          leadBotId={project.lead_bot_id}
          now={ctx.now}
          onReply={ctx.onReply}
          onOpenMeetings={() => ctx.onSelectTab("meetings")}
          summaryMeeting={ctx.summaryMeeting}
        />
      }
    />
  );
}

function Team({ ctx }: { readonly ctx: TabContext }): ReactElement {
  const { daemon, project } = ctx;
  return (
    <TeamView
      bots={ctx.bots}
      row={ctx.row}
      threads={ctx.threads}
      unread={daemon.unreadBots}
      failed={failedByBot(daemon.failedDeliveries)}
      activity={daemon.activityByBot}
      leadBotId={project.lead_bot_id}
      now={ctx.now}
      canControl={daemon.canControl}
      onOpenBot={(botId) => daemon.select({ kind: "bot", botId })}
      onCreateBot={() => void ctx.onCreateBot(project.id)}
      onDeleteBot={(botId) => void ctx.onDeleteBot(botId)}
    />
  );
}

function Settings({ ctx }: { readonly ctx: TabContext }): ReactElement {
  const { client, daemon, project } = ctx;
  const at = daemon.selection;
  return (
    <ProjectView
      client={client}
      project={project}
      bots={ctx.bots}
      connected={daemon.connected}
      canControl={daemon.canControl}
      onRename={ctx.onRenameProject}
      onSetLead={ctx.onSetProjectLead}
      onSetRepo={ctx.onSetProjectRepo}
      onDelete={ctx.onDeleteProject}
      onToast={ctx.addToast}
      ownerReview={client.capabilities.includes(PULL_REQUESTS)}
      focusOwnerReview={at.kind === "project" && at.section === "owner_review"}
    />
  );
}

/** The PR a selection opens, and whether as a re-check. */
function prOpening(at: Selection): { readonly pr?: number; readonly recheck?: boolean } {
  return at.kind === "project" ? { pr: at.pr, recheck: at.recheck } : {};
}

/** The Pull requests tab, on the PR Needs you or a link asked for. */
function Prs({ ctx }: { readonly ctx: TabContext }): ReactElement {
  const { client, daemon, project } = ctx;
  if (!client.capabilities.includes(PULL_REQUESTS)) {
    return <NeedsNewer what="Pull requests" />;
  }
  const { pr, recheck } = prOpening(daemon.selection);
  return (
    <PullRequestsView
      key={pr ?? "list"}
      client={client}
      project={project}
      connected={daemon.connected}
      now={ctx.now}
      initialNumber={pr}
      recheck={recheck}
      onOpenSettings={() =>
        daemon.select({
          kind: "project",
          projectId: project.id,
          tab: "settings",
          section: "owner_review",
        })
      }
    />
  );
}

/** The active tab's view. */
function TabView({ ctx }: { readonly ctx: TabContext }): ReactElement {
  const { client, daemon, project, bots } = ctx;
  const { connected, canControl } = daemon;
  switch (ctx.tab) {
    case "overview":
      return <Overview ctx={ctx} />;
    case "board":
      return (
        <BoardView
          client={client}
          project={project}
          bots={bots}
          connected={connected}
          canControl={canControl}
          addToast={ctx.addToast}
          openItem={daemon.selection.kind === "project" ? daemon.selection.item : undefined}
        />
      );
    case "prs":
      return <Prs ctx={ctx} />;
    case "team":
      return <Team ctx={ctx} />;
    case "releases":
      return (
        <ReleasesView
          client={client}
          project={project}
          bots={bots}
          connected={connected}
          canControl={canControl}
          addToast={ctx.addToast}
          onOpenDecision={(decisionId) => daemon.select({ kind: "control", decisionId })}
          onOpenNeedsYou={() => daemon.select({ kind: "control" })}
        />
      );
    case "meetings":
      return (
        <MeetingsView
          client={client}
          project={project}
          bots={bots}
          connected={connected}
          onReply={ctx.onReply}
        />
      );
    case "conversations":
      return client.capabilities.includes("agent_conversations") ? (
        <ConversationsView client={client} project={project} bots={bots} connected={connected} />
      ) : (
        <NeedsNewer what="Conversations" />
      );
    case "settings":
      return <Settings ctx={ctx} />;
  }
}

export default function ProjectPane(props: ProjectPaneProps): ReactElement {
  const { client, daemon, project, tab } = props;
  const { connected, select } = daemon;
  const bots = daemon.bots.filter((item) => item.project_id === project.id);
  const row = useProjectRow(client, project.id, connected);
  const threads = useOwnerThreads(client, connected).filter((t) => t.projectId === project.id);
  const now = useNow();
  const summaryMeeting = useMeeting(client, project.id, row?.latestSummary?.meetingId, connected);
  const onSelectTab = useCallback(
    (next: ProjectTab): void => {
      select({ kind: "project", projectId: project.id, tab: next });
    },
    [select, project.id],
  );
  const onOpenPr = useCallback(
    (pr: number, recheck: boolean): void => {
      select({ kind: "project", projectId: project.id, tab: "prs", pr, recheck });
    },
    [select, project.id],
  );

  return (
    <ProjectWindow
      key={project.id}
      project={project}
      botCount={bots.length}
      tab={tab}
      prs={client.capabilities.includes(PULL_REQUESTS)}
      row={row}
      onHome={() => select({ kind: "home" })}
      onSelectTab={onSelectTab}
    >
      <TabView ctx={{ ...props, bots, row, threads, now, onSelectTab, onOpenPr, summaryMeeting }} />
    </ProjectWindow>
  );
}
