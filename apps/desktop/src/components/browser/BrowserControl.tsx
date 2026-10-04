import { useEffect, useRef } from "react";
import type { KeyboardEvent, MouseEvent, ReactElement } from "react";
import type { BrowserFramePush, BrowserInputEvent } from "../../protocol/agents";
import { keyInput, modifierMask, mouseButton, pagePoint, wheelScale } from "./browserInput";

/** Fewest milliseconds between two mouse moves sent to the page. */
const MOVE_INTERVAL = 40;

/**
 * The bot's screen under the owner's mouse and keyboard: clicks, scrolling,
 * typing and pasting go to the page on show, as if the owner sat at it.
 */
export default function BrowserControl({
  frame,
  name,
  onInput,
}: {
  readonly frame: BrowserFramePush;
  readonly name: string;
  readonly onInput: (tabId: string, event: BrowserInputEvent) => void;
}): ReactElement {
  const surface = useRef<HTMLDivElement | null>(null);
  const screen = useRef<HTMLImageElement | null>(null);
  const held = useRef<"left" | "middle" | "right" | "none">("none");
  const last = useRef<{ x: number; y: number } | null>(null);
  const lastMove = useRef(0);

  useEffect(() => {
    surface.current?.focus();
  }, []);

  const send = (event: BrowserInputEvent): void => {
    onInput(frame.tab_id, event);
  };

  const point = (e: { clientX: number; clientY: number }): { x: number; y: number } | null => {
    const shown = screen.current;
    return shown === null
      ? null
      : pagePoint(shown.getBoundingClientRect(), frame, e.clientX, e.clientY);
  };

  const mouse = (e: MouseEvent, action: "down" | "up" | "move"): void => {
    e.preventDefault();
    if (action === "down") {
      surface.current?.focus();
    }
    if (action === "move") {
      const now = Date.now();
      if (now - lastMove.current < MOVE_INTERVAL) {
        return;
      }
      lastMove.current = now;
    }
    // A button let go off the page still lets go, where the mouse left it.
    const at = point(e) ?? (action === "up" ? last.current : null);
    if (at === null) {
      return;
    }
    last.current = at;
    const button =
      action === "move" || (action === "up" && held.current !== "none")
        ? held.current
        : mouseButton(e.button);
    held.current = action === "down" ? button : action === "up" ? "none" : held.current;
    send({
      kind: "mouse",
      action,
      ...at,
      button,
      clicks: Math.max(1, e.detail),
      modifiers: modifierMask(e),
    });
  };

  const key = (e: KeyboardEvent, action: "down" | "up"): void => {
    // The owner's paste shortcut stays the owner's: it fires `paste`, which
    // sends what the clipboard holds.
    if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "v") {
      return;
    }
    e.preventDefault();
    e.stopPropagation();
    const input = keyInput(e, action);
    if (input !== null) {
      send(input);
    }
  };

  // A remote screen is one widget the page's own controls live inside, so it
  // takes focus and the raw mouse and keyboard itself.
  /* oxlint-disable jsx-a11y/no-noninteractive-element-interactions, jsx-a11y/no-noninteractive-tabindex */
  return (
    <div
      ref={surface}
      className="browser-control"
      role="application"
      aria-label={`${name}'s browser, under your control`}
      tabIndex={0}
      onMouseDown={(e) => {
        mouse(e, "down");
      }}
      onMouseUp={(e) => {
        mouse(e, "up");
      }}
      onMouseMove={(e) => {
        mouse(e, "move");
      }}
      onMouseLeave={(e) => {
        if (held.current !== "none") {
          mouse(e, "up");
        }
      }}
      onContextMenu={(e) => {
        e.preventDefault();
      }}
      onWheel={(e) => {
        const at = point(e);
        if (at !== null) {
          const scale = wheelScale(e.deltaMode, frame.height);
          send({
            kind: "wheel",
            ...at,
            dx: e.deltaX * scale,
            dy: e.deltaY * scale,
            modifiers: modifierMask(e),
          });
        }
      }}
      onKeyDown={(e) => {
        key(e, "down");
      }}
      onKeyUp={(e) => {
        key(e, "up");
      }}
      onPaste={(e) => {
        e.preventDefault();
        const text = e.clipboardData.getData("text/plain");
        if (text !== "") {
          send({ kind: "text", text });
        }
      }}
    >
      <img
        ref={screen}
        className="browser-frame"
        src={`data:image/jpeg;base64,${frame.data}`}
        width={frame.width}
        height={frame.height}
        alt={`What ${name}'s browser shows`}
        draggable={false}
      />
    </div>
  );
  /* oxlint-enable jsx-a11y/no-noninteractive-element-interactions, jsx-a11y/no-noninteractive-tabindex */
}
