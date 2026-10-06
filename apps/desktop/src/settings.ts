import { isProjectTab } from "./app/selection";
import type { ProjectTab } from "./app/selection";
import type { Endpoint } from "./protocol/connection";
import { readStored, removeStored, writeStored } from "./storage";

const STORAGE_KEY = "connection";

export const DEFAULT_ENDPOINT: Endpoint = { host: "127.0.0.1", port: 49777 };

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function envString(key: string): string | undefined {
  const value: unknown = import.meta.env[key];
  return typeof value === "string" && value.length > 0 ? value : undefined;
}

/**
 * The workspace-private daemon `scripts/dev.sh` starts, injected at build time.
 * It seeds a fresh client so a dev window connects to its own daemon instead of
 * the installed one; anything stored in localStorage still wins.
 */
function devEndpoint(): Endpoint | undefined {
  const port = Number(envString("VITE_GRAVITY_DEV_PORT"));
  if (!Number.isInteger(port) || port <= 0) {
    return undefined;
  }
  return { host: envString("VITE_GRAVITY_DEV_HOST") ?? "127.0.0.1", port };
}

/** Loads the daemon endpoint from localStorage (a laptop can point at a Tailscale host). */
export function loadEndpoint(): Endpoint {
  try {
    const raw = readStored(STORAGE_KEY);
    if (raw === null) {
      return devEndpoint() ?? DEFAULT_ENDPOINT;
    }
    const parsed: unknown = JSON.parse(raw);
    if (
      isRecord(parsed) &&
      typeof parsed["host"] === "string" &&
      parsed["host"].length > 0 &&
      typeof parsed["port"] === "number" &&
      Number.isInteger(parsed["port"])
    ) {
      return { host: parsed["host"], port: parsed["port"] };
    }
  } catch {
    // fall through to default
  }
  return devEndpoint() ?? DEFAULT_ENDPOINT;
}

export function saveEndpoint(endpoint: Endpoint): void {
  try {
    writeStored(STORAGE_KEY, JSON.stringify(endpoint));
  } catch {
    // localStorage unavailable; setting is session-only
  }
}

const DEVICE_TOKEN_KEY = "device-token";

/**
 * A device-scoped token entered during remote setup. Preferred over the local
 * token file, which only exists on the machine running the daemon.
 */
export function loadDeviceToken(): string {
  const fallback = envString("VITE_GRAVITY_DEV_TOKEN") ?? "";
  try {
    return readStored(DEVICE_TOKEN_KEY) ?? fallback;
  } catch {
    return fallback;
  }
}

export function saveDeviceToken(token: string): void {
  try {
    if (token.length === 0) {
      removeStored(DEVICE_TOKEN_KEY);
    } else {
      writeStored(DEVICE_TOKEN_KEY, token);
    }
  } catch {
    // localStorage unavailable; token is session-only
  }
}

const SETUP_KEY = "setup-complete";

/** True once this install has connected to a daemon at least once. */
export function loadSetupComplete(): boolean {
  try {
    const raw = readStored(SETUP_KEY);
    if (raw === null) {
      // A dev build points at its own daemon already, so skip the wizard.
      return devEndpoint() !== undefined;
    }
    return raw === "true";
  } catch {
    return devEndpoint() !== undefined;
  }
}

export function markSetupComplete(): void {
  try {
    writeStored(SETUP_KEY, "true");
  } catch {
    // localStorage unavailable; the wizard may show again next launch
  }
}

const PROJECT_TAB_KEY = "project-tab.";

/** The tab a project window was last showing, so it reopens there. */
export function loadProjectTab(projectId: string): ProjectTab | undefined {
  try {
    const tab = readStored(PROJECT_TAB_KEY + projectId);
    return tab !== null && isProjectTab(tab) ? tab : undefined;
  } catch {
    return undefined;
  }
}

export function saveProjectTab(projectId: string, tab: ProjectTab): void {
  try {
    writeStored(PROJECT_TAB_KEY + projectId, tab);
  } catch {
    // localStorage unavailable; the window reopens on the Dashboard
  }
}

const BOT_INFO_PANEL_KEY = "bot-info-panel";

export interface BotInfoPanelSettings {
  readonly collapsed: boolean;
  readonly width: number;
}

export const DEFAULT_BOT_INFO_PANEL: BotInfoPanelSettings = {
  collapsed: false,
  width: 360,
};

export const MIN_BOT_INFO_PANEL_WIDTH = 280;

/** Loads the bot inspector layout shared by every bot view. */
export function loadBotInfoPanel(): BotInfoPanelSettings {
  try {
    const raw = readStored(BOT_INFO_PANEL_KEY);
    if (raw === null) {
      return DEFAULT_BOT_INFO_PANEL;
    }
    const parsed: unknown = JSON.parse(raw);
    if (
      isRecord(parsed) &&
      typeof parsed["collapsed"] === "boolean" &&
      typeof parsed["width"] === "number" &&
      Number.isFinite(parsed["width"]) &&
      parsed["width"] >= MIN_BOT_INFO_PANEL_WIDTH
    ) {
      return { collapsed: parsed["collapsed"], width: parsed["width"] };
    }
  } catch {
    // fall through to the default layout
  }
  return DEFAULT_BOT_INFO_PANEL;
}

export function saveBotInfoPanel(settings: BotInfoPanelSettings): void {
  try {
    writeStored(BOT_INFO_PANEL_KEY, JSON.stringify(settings));
  } catch {
    // localStorage unavailable; layout is session-only
  }
}

const UNREAD_KEY = "unread";

/**
 * One bot's badge state. `seenAt` is how far the user has caught up: the epoch
 * milliseconds of the newest item already read or already counted, so a
 * restarted client can tell what it has accounted for from what it has not.
 */
export interface UnreadEntry {
  readonly count: number;
  readonly seenAt: number;
}

export type UnreadState = Readonly<Record<string, UnreadEntry>>;

function entryFrom(value: unknown): UnreadEntry | undefined {
  if (
    isRecord(value) &&
    typeof value["count"] === "number" &&
    Number.isFinite(value["count"]) &&
    value["count"] >= 0 &&
    typeof value["seenAt"] === "number" &&
    Number.isFinite(value["seenAt"])
  ) {
    return { count: value["count"], seenAt: value["seenAt"] };
  }
  return undefined;
}

/**
 * Loads the unread badges. They live here rather than on the daemon because a
 * bot's replies never touch the message bus — the daemon only knows what a bot
 * has yet to consume, which says nothing about what the user has read.
 */
export function loadUnread(): UnreadState {
  try {
    const raw = readStored(UNREAD_KEY);
    if (raw === null) {
      return {};
    }
    const parsed: unknown = JSON.parse(raw);
    if (!isRecord(parsed)) {
      return {};
    }
    const state: Record<string, UnreadEntry> = {};
    for (const [botId, value] of Object.entries(parsed)) {
      const entry = entryFrom(value);
      if (entry !== undefined) {
        state[botId] = entry;
      }
    }
    return state;
  } catch {
    // fall through to no badges
  }
  return {};
}

export function saveUnread(state: UnreadState): void {
  try {
    writeStored(UNREAD_KEY, JSON.stringify(state));
  } catch {
    // localStorage unavailable; badges are session-only
  }
}
