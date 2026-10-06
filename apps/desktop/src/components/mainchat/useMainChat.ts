// The main chat's state (UX-024 §2, H-133 U3): open or closed, the bot it is
// "To", and a report being answered. ⌘J opens it from anywhere; Reply on a
// report opens it on that bot with the report quoted.

import { useCallback, useEffect, useState } from "react";

interface MainChatState {
  readonly open: boolean;
  /** The bot the next message goes to; null until one is picked. */
  readonly botId: string | null;
  /** The report being answered, quoted above the message. */
  readonly quote: string | null;
}

export interface MainChatApi extends MainChatState {
  /** Opens on a bot, or on whoever it was last on. */
  readonly openOn: (botId?: string, quote?: string) => void;
  readonly pick: (botId: string) => void;
  readonly clearQuote: () => void;
  readonly close: () => void;
}

const CLOSED: MainChatState = { open: false, botId: null, quote: null };

/**
 * ⌘J toggles the panel. `fallbackBot` is who "To" starts on when the owner
 * opens it without picking: the bot on screen, else the project's lead.
 */
export function useMainChat(fallbackBot: () => string | null): MainChatApi {
  const [state, setState] = useState<MainChatState>(CLOSED);

  const openOn = useCallback(
    (botId?: string, quote?: string): void => {
      setState((current) => ({
        open: true,
        botId: botId ?? current.botId ?? fallbackBot(),
        quote: quote ?? null,
      }));
    },
    [fallbackBot],
  );

  useEffect(() => {
    const onKey = (event: KeyboardEvent): void => {
      if (!(event.metaKey || event.ctrlKey) || event.shiftKey || event.altKey) {
        return;
      }
      if (event.key.toLowerCase() === "j") {
        event.preventDefault();
        event.stopPropagation();
        setState((current) =>
          current.open
            ? { ...current, open: false, quote: null }
            : { ...current, open: true, botId: current.botId ?? fallbackBot() },
        );
      }
    };
    // Captured before a focused terminal can turn it into input.
    window.addEventListener("keydown", onKey, true);
    return () => {
      window.removeEventListener("keydown", onKey, true);
    };
  }, [fallbackBot]);

  return {
    ...state,
    openOn,
    pick: useCallback((botId: string): void => {
      setState((current) => ({ ...current, botId, quote: null }));
    }, []),
    clearQuote: useCallback((): void => {
      setState((current) => ({ ...current, quote: null }));
    }, []),
    close: useCallback((): void => {
      setState((current) => ({ ...current, open: false, quote: null }));
    }, []),
  };
}
