import { useEffect } from "react";

/** Text the owner is typing into, where ⌘[ belongs to the field. */
function typing(target: EventTarget | null): boolean {
  return (
    target instanceof HTMLElement &&
    (target.tagName === "TEXTAREA" ||
      target.tagName === "INPUT" ||
      target.isContentEditable ||
      target.closest(".xterm") !== null)
  );
}

/**
 * ⌘[ and ⌘] (and the mouse's back and forward buttons) go through the app
 * history (UX-035 §5). The card drawer takes them first while it's open.
 */
export function useHistoryKeys(go: (direction: -1 | 1) => void): void {
  useEffect(() => {
    const onKey = (event: KeyboardEvent): void => {
      if (!event.metaKey || event.defaultPrevented || typing(event.target)) {
        return;
      }
      if (event.key === "[" || event.key === "]") {
        event.preventDefault();
        go(event.key === "[" ? -1 : 1);
      }
    };
    const onMouse = (event: MouseEvent): void => {
      if (event.button === 3 || event.button === 4) {
        event.preventDefault();
        go(event.button === 3 ? -1 : 1);
      }
    };
    window.addEventListener("keydown", onKey);
    window.addEventListener("mouseup", onMouse);
    return () => {
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("mouseup", onMouse);
    };
  }, [go]);
}
