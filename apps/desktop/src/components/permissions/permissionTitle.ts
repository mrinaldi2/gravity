import type { PermissionRequest, TerminalOrigin } from "../../protocol/chat";
import { toolDisplayName } from "../../toolNames";

/**
 * What a CLI owner command's card is filed under (H-044 T4): a command run
 * in a terminal that wants to act as the owner, not a bot's tool.
 */
const TERMINAL = "terminal";

export function isTerminal(request: PermissionRequest): boolean {
  return request.bot_id === TERMINAL;
}

/** The terminal card's copy (UX-014). The daemon's `summary` is never shown. */
export const TERMINAL_TITLE = "A terminal command wants to act as you";

export const TERMINAL_BODY =
  "If you allow it, this one command runs with your owner rights: it can do anything you can " +
  "do in The Hermes, including approving releases and deleting bots. Bots can run terminal " +
  "commands too. Allow it only if you just ran it yourself.";

/** "Desktop Dev wants to run Bash", or the terminal's own line. */
export function permissionTitle(request: PermissionRequest, botName: string | undefined): string {
  if (isTerminal(request)) {
    return TERMINAL_TITLE;
  }
  return `${botName ?? "A bot"} wants to run ${toolDisplayName(request.tool)}`;
}

/** The terminal card's facts; a daemon from before UX-014 sent none. */
function terminalOrigin(request: PermissionRequest): TerminalOrigin | undefined {
  return isTerminal(request) ? request.origin : undefined;
}

/** `/Users/me/x` → `~/x`, the way the owner reads their own folders. */
function shortPath(path: string): string {
  return path.replace(/^(\/Users\/[^/]+|\/home\/[^/]+|[A-Za-z]:\\Users\\[^\\]+)(?=[/\\]|$)/, "~");
}

/** "From hermesd in Terminal · in ~/Developer/gravity · process 4242". */
export function originLine(origin: TerminalOrigin): string {
  const where = origin.cwd === undefined ? [] : [`in ${shortPath(origin.cwd)}`];
  const pid = `process ${origin.pid}`;
  if (origin.process === undefined && origin.launched_from === undefined) {
    const known = [`From ${pid}`, ...where].join(" · ");
    return `${known}. The Hermes couldn't tell which app started it.`;
  }
  const who = [origin.process, origin.launched_from].filter((part) => part !== undefined);
  return [`From ${who.join(" in ")}`, ...where, pid].join(" · ");
}

export function botWarning(bot: string): string {
  return `⚠ It was started inside ${bot}'s workspace, so a bot is probably asking, not you.`;
}

/** What a toast says under the title: the terminal's own body, or the bot's summary. */
export function permissionDetail(request: PermissionRequest): string {
  const origin = terminalOrigin(request);
  if (origin === undefined) {
    return request.summary;
  }
  const started = origin.bot === undefined ? "" : `Started in ${origin.bot}'s workspace. `;
  return `${started}${origin.command} · open The Hermes to allow or deny it.`;
}
