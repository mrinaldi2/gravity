import { useCallback, useState } from "react";
import type { ReactElement } from "react";
import type { BrowserFramePush, BrowserInputEvent, BrowserTabsPush } from "../../protocol/agents";
import type { DaemonApi } from "../../protocol/api";
import type { Bot } from "../../protocol/entities";
import BrowserActivityList from "./BrowserActivityList";
import BrowserControl from "./BrowserControl";
import { useBrowserActivity } from "./useBrowserActivity";
import type { BrowserWatch } from "./useBrowserWatch";

interface BrowserPaneProps {
  readonly client: DaemonApi;
  readonly bot: Bot;
  /** The bot's browser as it streams (see `useBrowserWatch`). */
  readonly watch: BrowserWatch;
  readonly connected: boolean;
  /** Whether the owner may take the mouse and keyboard (`browser_input`). */
  readonly canControl: boolean;
}

/** Where the bot's own browser stands, said above the screen. */
function chromeNote(bot: Bot): string {
  if (bot.peer != null) {
    return `${bot.name} runs on ${bot.peer.name}; its browser streams from there.`;
  }
  return bot.user_chrome === true
    ? `${bot.name} uses a browser of its own, and may also use your Chrome.`
    : `${bot.name} uses a browser of its own; your Chrome is off limits.`;
}

/** Each of the bot's tabs; picking one shows it, "Follow" goes back to the bot's. */
export function TabStrip({
  tabs,
  onPick,
}: {
  readonly tabs: BrowserTabsPush;
  readonly onPick: (tabId: string | null) => void;
}): ReactElement {
  return (
    <div className="browser-tabs" role="tablist" aria-label="Browser tabs">
      {tabs.tabs.map((tab) => (
        <button
          key={tab.id}
          type="button"
          role="tab"
          aria-selected={tab.id === tabs.active}
          className={`browser-tab ${tab.id === tabs.active ? "browser-tab-active" : ""}`}
          title={tab.url}
          onClick={() => {
            onPick(tab.id);
          }}
        >
          {tab.title === "" ? tab.url : tab.title}
        </button>
      ))}
      <button
        type="button"
        className={`browser-follow ${tabs.following === false ? "" : "browser-follow-on"}`}
        aria-pressed={tabs.following !== false}
        title="Show whichever tab the bot is using"
        onClick={() => {
          onPick(null);
        }}
      >
        Follow bot
      </button>
    </div>
  );
}

/** The tab on show, as its latest screen; under the owner's control with `onInput`. */
export function BrowserScreen({
  tabs,
  frame,
  name,
  onInput,
}: {
  readonly tabs: BrowserTabsPush | null;
  readonly frame: BrowserFramePush | null;
  readonly name: string;
  readonly onInput?: (tabId: string, event: BrowserInputEvent) => void;
}): ReactElement {
  if (tabs === null) {
    return <div className="browser-empty muted">Looking for the browser…</div>;
  }
  if (!tabs.open) {
    return (
      <div className="browser-empty muted">
        {tabs.reason ?? `${name}'s browser is closed. It opens when ${name} first browses.`}
      </div>
    );
  }
  const url = tabs.tabs.find((tab) => tab.id === tabs.active)?.url ?? "";
  return (
    <div className="browser-screen">
      <div className="browser-url" title={url}>
        {url}
      </div>
      {frame !== null && frame.tab_id === tabs.active && onInput !== undefined ? (
        <BrowserControl frame={frame} name={name} onInput={onInput} />
      ) : frame !== null && frame.tab_id === tabs.active ? (
        <img
          className="browser-frame"
          src={`data:image/jpeg;base64,${frame.data}`}
          width={frame.width}
          height={frame.height}
          alt={`What ${name}'s browser shows`}
        />
      ) : (
        <div className="browser-empty muted">Waiting for the page…</div>
      )}
    </div>
  );
}

/** What the browser is, and the switch handing the owner its mouse and keyboard. */
function ControlBar({
  bot,
  offered,
  controlling,
  onToggle,
}: {
  readonly bot: Bot;
  readonly offered: boolean;
  readonly controlling: boolean;
  readonly onToggle: () => void;
}): ReactElement {
  return (
    <div className="browser-bar">
      <div className="browser-note muted">
        {controlling
          ? `Your clicks and typing go to the page; ${bot.name} may act on it too.`
          : chromeNote(bot)}
      </div>
      {offered ? (
        <button
          type="button"
          className={`browser-take ${controlling ? "browser-take-on" : ""}`}
          aria-pressed={controlling}
          title="Use the mouse and keyboard on the page, to sign in or get past a step"
          onClick={onToggle}
        >
          {controlling ? "Give back control" : "Take control"}
        </button>
      ) : null}
    </div>
  );
}

/**
 * The bot's own browser, live: its tabs, the page on show, and every
 * browser action it took with the turn that led to it.
 */
export default function BrowserPane({
  client,
  bot,
  watch,
  connected,
  canControl,
}: BrowserPaneProps): ReactElement {
  const activity = useBrowserActivity(client, bot.id, connected);
  const [control, setControl] = useState(false);
  const input = useCallback(
    (tabId: string, event: BrowserInputEvent): void => {
      client.fire({ type: "browser_input", bot_id: bot.id, tab_id: tabId, event });
    },
    [client, bot.id],
  );
  const open = watch.tabs !== null && watch.tabs.open;
  const controlling = control && canControl && open && connected;
  return (
    <div className="browser-pane">
      <div className="browser-main">
        <ControlBar
          bot={bot}
          offered={canControl && open}
          controlling={controlling}
          onToggle={() => {
            setControl(!controlling);
          }}
        />
        {watch.tabs !== null && open ? <TabStrip tabs={watch.tabs} onPick={watch.pick} /> : null}
        {watch.error === null ? null : <div className="chat-note chat-error">{watch.error}</div>}
        <BrowserScreen
          tabs={watch.tabs}
          frame={watch.frame}
          name={bot.name}
          {...(controlling ? { onInput: input } : {})}
        />
      </div>
      <BrowserActivityList activity={activity.items} error={activity.error} />
    </div>
  );
}
