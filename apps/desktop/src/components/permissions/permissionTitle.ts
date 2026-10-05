import type { PermissionRequest } from "../../protocol/chat";
import { toolDisplayName } from "../../toolNames";

/**
 * What a CLI owner command's card is filed under (H-044 T4): a command run
 * in a terminal that wants to act as the owner, not a bot's tool.
 */
const TERMINAL = "terminal";

export function isTerminal(request: PermissionRequest): boolean {
  return request.bot_id === TERMINAL;
}

/** "Desktop Dev wants to run Bash", or the terminal's own line. */
export function permissionTitle(request: PermissionRequest, botName: string | undefined): string {
  if (isTerminal(request)) {
    return "A command in Terminal wants to act as you";
  }
  return `${botName ?? "A bot"} wants to run ${toolDisplayName(request.tool)}`;
}
