import { useCallback, useState } from "react";
import type { ReactElement } from "react";
import { useDaemonActions } from "./app/useDaemonActions";
import { useFirstRunSetup } from "./app/useFirstRunSetup";
import type { FirstRunSetup } from "./app/useFirstRunSetup";
import { useDaemonState } from "./app/useDaemonState";
import type { DaemonState } from "./app/useDaemonState";
import { useDesktopShell } from "./app/useDesktopShell";
import { usePermissionInbox } from "./app/usePermissionInbox";
import { useOpenBot } from "./app/useOpenBot";
import { useOverlays } from "./app/useOverlays";
import type { OverlaysApi } from "./app/useOverlays";
import { usePaletteActions } from "./app/usePaletteActions";
import { useToasts } from "./app/useToasts";
import type { AddToast } from "./app/useToasts";
import { useServiceRecovery } from "./app/useServiceRecovery";
import { useUpdates } from "./app/useUpdates";
import CardsLayer from "./components/cards/CardsLayer";
import CommandPalette from "./components/CommandPalette";
import Rail from "./components/home/Rail";
import MainChat from "./components/mainchat/MainChat";
import { useAppChat } from "./components/mainchat/useAppChat";
import HomeMigrationConfirm from "./components/overlay/HomeMigrationConfirm";
import MainPane from "./components/MainPane";
import SearchOverlay from "./components/SearchOverlay";
import QuiesceLayer from "./components/service/QuiesceBanner";
import ServiceRecoveryBanner from "./components/service/ServiceRecoveryBanner";
import SettingsOverlay from "./components/settings/SettingsOverlay";
import SetupScreen from "./components/setup/SetupScreen";
import Toasts from "./components/Toasts";
import type { Toast } from "./components/Toasts";
import type { DaemonApi } from "./protocol/api";
import type { Endpoint } from "./protocol/connection";
import { DaemonClient } from "./protocol/client";
import { loadEndpoint } from "./settings";
import { readClientToken } from "./token";

interface SettingsLayerProps {
  readonly client: DaemonApi;
  readonly daemon: DaemonState;
  readonly overlays: OverlaysApi;
  readonly addToast: AddToast;
}

function SettingsLayer({
  client,
  daemon,
  overlays,
  addToast,
}: SettingsLayerProps): ReactElement | null {
  if (!overlays.settingsOpen) {
    return null;
  }
  return (
    <SettingsOverlay
      client={client}
      daemon={daemon}
      category={overlays.settingsCategory}
      addToast={addToast}
      onSelectCategory={overlays.selectSettingsCategory}
      onClose={overlays.closeSettings}
    />
  );
}

interface ServiceRecoveryLayerProps {
  readonly endpoint: Endpoint;
  readonly addToast: AddToast;
}

/** The launch-time service check and, when it found something, its one fix. */
function ServiceRecoveryLayer({
  endpoint,
  addToast,
}: ServiceRecoveryLayerProps): ReactElement | null {
  const recovery = useServiceRecovery(endpoint, addToast);
  if (recovery.offer === null) {
    return null;
  }
  return (
    <ServiceRecoveryBanner
      offer={recovery.offer}
      installing={recovery.installing}
      error={recovery.error}
      onInstall={recovery.install}
      onDismiss={recovery.dismiss}
    />
  );
}

interface SetupGateProps {
  readonly setup: FirstRunSetup;
  readonly toasts: readonly Toast[];
  readonly onDismissToast: (id: number) => void;
  readonly children: ReactElement;
}

function SetupGate({ setup, toasts, onDismissToast, children }: SetupGateProps): ReactElement {
  if (!setup.showWizard) {
    return children;
  }
  return (
    <div className="app">
      <SetupScreen onConnect={setup.connectToDaemon} />
      <Toasts toasts={toasts} onDismiss={onDismissToast} />
      <HomeMigrationConfirm />
    </div>
  );
}

export default function App(): ReactElement {
  const [client] = useState(() => new DaemonClient(loadEndpoint(), readClientToken));
  const { toasts, addToast, dismissToast } = useToasts();
  const daemon = useDaemonState(client, addToast);
  const overlays = useOverlays();
  const { select, bots, canControl, connected } = daemon;

  useUpdates(addToast, daemon.status, daemon.endpoint);

  const actions = useDaemonActions({
    client,
    addToast,
    refreshAll: daemon.refreshAll,
    applyBotUpdate: daemon.applyBotUpdate,
    select,
  });

  /** Opens the bot whose DM thread backs a conversation. */
  const openConversation = useCallback(
    (conversationId: string): void => {
      const conversation = daemon.findConversation(conversationId);
      if (conversation === undefined) {
        addToast("warn", "Conversation not found", "The conversation is no longer available.");
        return;
      }
      select({ kind: "bot", botId: conversation.bot_id });
    },
    [addToast, daemon, select],
  );

  const { openBot, openBotChat } = useOpenBot(select);

  const inbox = usePermissionInbox({
    client,
    connected: daemon.connected,
    bots,
    selection: daemon.selection,
    select,
    addToast,
  });
  const pending = inbox.counts;

  const paletteActions = usePaletteActions({
    bots,
    select,
    openSettings: overlays.openSettings,
  });

  const { chat, threads, asking } = useAppChat(
    client,
    connected,
    daemon.selection,
    daemon.projects,
  );

  useDesktopShell(daemon.unreadBots, pending.total, addToast);

  const setup = useFirstRunSetup(client, daemon.changeEndpoint);

  return (
    <SetupGate setup={setup} toasts={toasts} onDismissToast={dismissToast}>
      <CardsLayer client={client} daemon={daemon}>
        <div className="app">
          <Rail
            selection={daemon.selection}
            needsYou={pending.total}
            status={daemon.status}
            endpoint={daemon.endpoint}
            canControl={canControl}
            chatOpen={chat.open}
            chatAsking={asking}
            onSelect={select}
            onToggleChat={() => {
              if (chat.open) {
                chat.close();
              } else {
                chat.openOn();
              }
            }}
            onOpenSettings={overlays.openSettings}
          />
          <main className="main">
            <ServiceRecoveryLayer endpoint={daemon.endpoint} addToast={addToast} />
            <QuiesceLayer client={client} connected={connected} addToast={addToast} />
            <MainPane
              client={client}
              daemon={daemon}
              addToast={addToast}
              onCreateProject={actions.createProjectWithBot}
              onRenameProject={actions.renameProject}
              onSetProjectLead={actions.setProjectLead}
              onSetProjectRepo={actions.setProjectRepo}
              onDeleteProject={actions.deleteProject}
              onCreateBot={actions.createBot}
              onDeleteBot={actions.deleteBot}
              permissions={inbox.permissions}
              onOpenBot={openBot}
              onReply={chat.openOn}
            />
          </main>
          <MainChat
            client={client}
            connected={connected}
            canControl={canControl}
            bots={bots}
            projects={daemon.projects}
            threads={threads}
            chat={chat}
            addToast={addToast}
            onOpenBot={openBotChat}
          />
          {overlays.paletteOpen ? (
            <CommandPalette
              actions={paletteActions}
              onSearch={overlays.openSearch}
              onClose={overlays.closePalette}
            />
          ) : null}
          {overlays.searchOpen ? (
            <SearchOverlay
              client={client}
              conversations={daemon.conversations}
              bots={bots}
              projects={daemon.projects}
              connected={connected}
              initialQuery={overlays.searchQuery}
              onOpenBot={openBot}
              onOpenConversation={openConversation}
              onClose={overlays.closeSearch}
            />
          ) : null}
          <SettingsLayer client={client} daemon={daemon} overlays={overlays} addToast={addToast} />
          <Toasts toasts={toasts} onDismiss={dismissToast} />
          <HomeMigrationConfirm />
        </div>
      </CardsLayer>
    </SetupGate>
  );
}
