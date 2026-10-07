import type { ReactElement } from "react";
import { isStopped } from "../../app/bots";
import type { AddToast } from "../../app/useToasts";
import type { DaemonApi } from "../../protocol/api";
import type { Bot, Routine } from "../../protocol/entities";
import ChatPane from "../chat/ChatPane";
import LinkedChat from "../chat/LinkedChat";
import PermissionCards from "../permissions/PermissionCards";
import { usePermissions } from "../permissions/usePermissions";
import BrowserPane from "../browser/BrowserPane";
import type { BrowserWatch } from "../browser/useBrowserWatch";
import RoutinesPanel from "../RoutinesPanel";
import TerminalPane from "../TerminalPane";
import type { BotTab } from "./BotTabs";
import BotChat from "./BotChat";
import ReportsPane from "./ReportsPane";

interface BotPanesProps {
  readonly client: DaemonApi;
  readonly bot: Bot;
  readonly bots: readonly Bot[];
  readonly tabs: readonly BotTab[];
  readonly active: BotTab;
  /** The bot's browser, streamed while the bot is selected. */
  readonly browser: BrowserWatch;
  readonly connected: boolean;
  readonly canControl: boolean;
  readonly onOpenFile: (path: string) => void;
  readonly onOpenDecision?: (decisionId: string) => void;
  readonly onRoutinesChanged: (botId: string, routines: readonly Routine[]) => void;
  readonly onToast: AddToast;
  /** Reply on Reports: answers the bot, quoting what it said. */
  readonly onReply: (quote: string) => void;
  /** Moves to another tab, e.g. from Activity to Chat. */
  readonly onSelectTab: (tab: BotTab) => void;
}

/** Why the owner cannot write to a bot right now, or null when they can. */
function writeBlockedReason(connected: boolean, canControl: boolean, bot: Bot): string | null {
  if (!connected) {
    return "Not connected to the Hermes service";
  }
  if (!canControl) {
    return "Read-only connection";
  }
  return isStopped(bot) ? `${bot.name} is stopped` : null;
}

/** Where a linked bot runs, said above its chat. */
function machineNote(bot: Bot): string {
  const machine = bot.peer?.name ?? "another computer";
  const offline =
    bot.peer?.online === false ? " It is offline, so its chat loads once it is back." : "";
  return `${bot.name} runs on ${machine}; its chat is read from there.${offline}`;
}

function paneClass(shown: boolean): string {
  return shown ? "tab-pane" : "tab-pane tab-pane-hidden";
}

/** Activity's caption (UX-033): what it holds, and where the conversation is. */
function ActivityCaption(props: {
  readonly bot: Bot;
  readonly onOpenChat: () => void;
}): ReactElement {
  return (
    <>
      Everything {props.bot.name} did: tasks, messages from other bots and the steps it took. Your
      conversation is in{" "}
      <button type="button" className="chat-link" onClick={props.onOpenChat}>
        Chat
      </button>
      .
    </>
  );
}

/**
 * The bot's session read from its transcript: a linked bot's from its
 * machine, else its own. It is Activity where Chat is the owner thread
 * (H-192), and the Chat tab itself on an older service.
 */
function TranscriptTab(
  props: Pick<
    BotPanesProps,
    "client" | "bot" | "connected" | "canControl" | "onOpenFile" | "onOpenDecision"
  > & {
    readonly shown: boolean;
    /** Opens Chat; given only when this is Activity. */
    readonly onOpenChat?: () => void;
  },
): ReactElement {
  const { client, bot, connected, onOpenChat } = props;
  const writeBlocked = writeBlockedReason(connected, props.canControl, bot);
  const linked = bot.peer != null;
  return (
    <div className={paneClass(props.shown)}>
      {linked && !client.capabilities.includes("peer_chat") ? (
        <LinkedChat client={client} bot={bot} connected={connected} writeBlocked={writeBlocked} />
      ) : (
        <ChatPane
          client={client}
          bot={bot}
          connected={connected}
          writeBlocked={writeBlocked}
          onOpenFile={props.onOpenFile}
          onOpenDecision={props.onOpenDecision}
          active={props.shown}
          note={linked ? machineNote(bot) : undefined}
          caption={
            onOpenChat === undefined ? undefined : (
              <ActivityCaption bot={bot} onOpenChat={onOpenChat} />
            )
          }
          composerNote={onOpenChat === undefined ? undefined : "Shows in Chat too."}
          onOpenChat={onOpenChat}
        />
      )}
    </div>
  );
}

/** On a service without owner threads, Chat is still the transcript: say so (UX-033). */
function OldServiceNote(props: {
  readonly client: DaemonApi;
  readonly active: BotTab;
}): ReactElement | null {
  if (props.active !== "chat" || props.client.capabilities.includes("owner_threads")) {
    return null;
  }
  return (
    <div className="chat-note bot-old-service">
      This computer's Hermes service shows Chat and Activity together. Update it to keep your
      conversation separate.
    </div>
  );
}

/**
 * Chat and Activity (H-192): where the service keeps owner threads, Chat is
 * the owner thread and the transcript is Activity; otherwise Chat is the
 * transcript.
 */
function TalkPanes(
  props: Pick<
    BotPanesProps,
    | "client"
    | "bot"
    | "tabs"
    | "active"
    | "connected"
    | "canControl"
    | "onOpenFile"
    | "onOpenDecision"
    | "onSelectTab"
  >,
): ReactElement {
  const { client, bot, tabs, active, connected, canControl } = props;
  const transcript = {
    client,
    bot,
    connected,
    canControl,
    onOpenFile: props.onOpenFile,
    onOpenDecision: props.onOpenDecision,
  };
  if (!tabs.includes("activity")) {
    return tabs.includes("chat") ? (
      <TranscriptTab {...transcript} shown={active === "chat"} />
    ) : (
      <></>
    );
  }
  return (
    <>
      <div className={paneClass(active === "chat")}>
        <BotChat
          client={client}
          bot={bot}
          connected={connected}
          writeBlocked={writeBlockedReason(connected, canControl, bot)}
          onOpenActivity={() => {
            props.onSelectTab("activity");
          }}
        />
      </div>
      <TranscriptTab
        {...transcript}
        shown={active === "activity"}
        onOpenChat={() => {
          props.onSelectTab("chat");
        }}
      />
    </>
  );
}

/**
 * The bot's tab bodies. Chat and terminal stay mounted when hidden, so
 * switching back keeps the loaded conversation and the terminal buffer.
 */
export default function BotPanes(props: BotPanesProps): ReactElement {
  const { client, bot, tabs, active, connected, canControl, onToast } = props;
  const linked = bot.peer != null;
  const permissions = usePermissions(client, bot.id, connected && !linked);
  return (
    <>
      {/* Above every tab: a prompt waits whether the owner reads the chat or the terminal. */}
      <PermissionCards permissions={permissions} canAnswer={connected && canControl} />
      <OldServiceNote client={client} active={active} />
      <div className="bot-view-body">
        {active === "reports" ? (
          <ReportsPane client={client} bot={bot} connected={connected} onReply={props.onReply} />
        ) : null}
        <TalkPanes {...props} />
        {tabs.includes("terminal") ? (
          <div className={paneClass(active === "terminal")}>
            {/* The terminal belongs to the user: any `control` connection may type while the bot runs. */}
            <TerminalPane
              client={client}
              botId={bot.id}
              canWrite={canControl && !isStopped(bot)}
              onToast={onToast}
            />
          </div>
        ) : null}
        {tabs.includes("browser") ? (
          // Mounted while hidden, so the live screen and the activity log
          // are there the moment the tab is opened.
          <div className={paneClass(active === "browser")}>
            <BrowserPane
              client={client}
              bot={bot}
              watch={props.browser}
              connected={connected}
              canControl={canControl && client.capabilities.includes("browser_input")}
            />
          </div>
        ) : null}
        {active === "routines" ? (
          <div className="tab-pane tab-pane-scroll">
            <RoutinesPanel
              client={client}
              bot={bot}
              bots={props.bots}
              connected={connected}
              canControl={canControl}
              onRoutinesChanged={props.onRoutinesChanged}
              onToast={onToast}
            />
          </div>
        ) : null}
      </div>
    </>
  );
}
