import { useEffect } from "react";
import { PRIMARY_TABS } from "../../app/selection";
import type { ProjectTab } from "../../app/selection";

/**
 * ⌘1…⌘5 switch the project window's tabs, the same pattern as a bot's ⌘1…⌘4.
 * Registered in the capture phase so a focused field cannot swallow them.
 */
export function useProjectKeys(onSelect: (tab: ProjectTab) => void): void {
  useEffect(() => {
    const onKey = (event: KeyboardEvent): void => {
      if (!(event.metaKey || event.ctrlKey) || event.altKey || event.shiftKey) {
        return;
      }
      const tab = PRIMARY_TABS[Number.parseInt(event.key, 10) - 1];
      if (tab !== undefined) {
        event.preventDefault();
        onSelect(tab);
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => {
      window.removeEventListener("keydown", onKey, true);
    };
  }, [onSelect]);
}
