// Stories that show a screen one click in (a filter, a section, a dialog).

import { useEffect } from "react";
import type { RefObject } from "react";

/** Clicks the button whose text starts with `label` once it shows. */
export function useClickOnce(box: RefObject<HTMLElement | null>, label: string | undefined): void {
  useEffect(() => {
    if (label === undefined) {
      return undefined;
    }
    const timer = setInterval(() => {
      const button = [...(box.current?.querySelectorAll<HTMLButtonElement>("button") ?? [])].find(
        (b) => b.textContent?.startsWith(label) === true,
      );
      if (button !== undefined) {
        button.click();
        clearInterval(timer);
      }
    }, 10);
    return () => clearInterval(timer);
  }, [box, label]);
}
