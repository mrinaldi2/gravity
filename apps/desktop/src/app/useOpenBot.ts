import { useCallback, useRef } from "react";
import type { Selection } from "./selection";

/** The ways to open a bot's page. */
export interface OpenBot {
  /** On its first tab. */
  readonly openBot: (botId: string) => void;
  /** On its Chat: from the main chat, the owner lands where they were (H-192). */
  readonly openBotChat: (botId: string) => void;
}

export function useOpenBot(select: (selection: Selection) => void): OpenBot {
  const openBot = useCallback(
    (botId: string): void => {
      select({ kind: "bot", botId });
    },
    [select],
  );
  // Each press is a new ask: the page is keyed on it (UX-034).
  const presses = useRef(0);
  const openBotChat = useCallback(
    (botId: string): void => {
      presses.current += 1;
      select({ kind: "bot", botId, tab: "chat", press: presses.current });
    },
    [select],
  );
  return { openBot, openBotChat };
}
