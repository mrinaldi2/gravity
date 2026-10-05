// Esc for a side drawer (U4, UX-011/UX-012): the drawer closes only when no
// menu or dialog is open anywhere, global overlays included, and nothing
// else has taken the key first.

import { useEffect } from "react";

/** Whether this Esc is the drawer's to take. */
export function escapeIsTheDrawers(event: KeyboardEvent): boolean {
  return (
    event.key === "Escape" &&
    !event.defaultPrevented &&
    document.querySelector('[role="menu"], [role="dialog"]') === null
  );
}

export function useDrawerEscape(onClose: () => void): void {
  useEffect(() => {
    const onKey = (event: KeyboardEvent): void => {
      if (escapeIsTheDrawers(event)) {
        onClose();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);
}
