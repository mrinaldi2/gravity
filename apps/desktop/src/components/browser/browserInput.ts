// Turning the owner's mouse and keyboard on a bot's screen into what the
// daemon passes to the page (`browser_input`).

import type { BrowserInputEvent } from "../../protocol/agents";

interface Box {
  readonly left: number;
  readonly top: number;
  readonly width: number;
  readonly height: number;
}

interface Size {
  readonly width: number;
  readonly height: number;
}

/**
 * Where a point on the shown screen lands on the page, in its CSS pixels, or
 * null off the page. The screen is scaled to fit its box, top centered
 * (`object-fit: contain; object-position: top center`).
 */
export function pagePoint(
  box: Box,
  page: Size,
  clientX: number,
  clientY: number,
): { readonly x: number; readonly y: number } | null {
  if (page.width <= 0 || page.height <= 0 || box.width <= 0 || box.height <= 0) {
    return null;
  }
  const scale = Math.min(box.width / page.width, box.height / page.height);
  const left = box.left + (box.width - page.width * scale) / 2;
  const x = (clientX - left) / scale;
  const y = (clientY - box.top) / scale;
  if (x < 0 || y < 0 || x > page.width || y > page.height) {
    return null;
  }
  return { x: Math.round(x), y: Math.round(y) };
}

interface Modified {
  readonly altKey: boolean;
  readonly ctrlKey: boolean;
  readonly metaKey: boolean;
  readonly shiftKey: boolean;
}

/** DevTools' modifier mask: Alt 1, Ctrl 2, Meta 4, Shift 8. */
export function modifierMask(e: Modified): number {
  return (e.altKey ? 1 : 0) | (e.ctrlKey ? 2 : 0) | (e.metaKey ? 4 : 0) | (e.shiftKey ? 8 : 0);
}

/** Windows key codes, which Chrome needs for keys that type nothing. */
const KEY_CODES: Readonly<Record<string, number>> = {
  Backspace: 8,
  Tab: 9,
  Enter: 13,
  Shift: 16,
  Control: 17,
  Alt: 18,
  Escape: 27,
  " ": 32,
  PageUp: 33,
  PageDown: 34,
  End: 35,
  Home: 36,
  ArrowLeft: 37,
  ArrowUp: 38,
  ArrowRight: 39,
  ArrowDown: 40,
  Delete: 46,
  Meta: 91,
};

function keyCode(key: string, code: string): number {
  const named = KEY_CODES[key];
  if (named !== undefined) {
    return named;
  }
  const letter = /^Key([A-Z])$/.exec(code)?.[1];
  if (letter !== undefined) {
    return letter.charCodeAt(0);
  }
  const digit = /^Digit([0-9])$/.exec(code)?.[1];
  return digit === undefined ? 0 : digit.charCodeAt(0);
}

interface KeyLike extends Modified {
  readonly key: string;
  readonly code: string;
}

/**
 * A key going down or up, or null for one that is only part of composing a
 * character (a dead key, an input method).
 */
export function keyInput(e: KeyLike, action: "down" | "up"): BrowserInputEvent | null {
  if (e.key === "Dead" || e.key === "Process" || e.key === "Unidentified") {
    return null;
  }
  const shortcut = e.ctrlKey || e.metaKey;
  const types = [...e.key].length === 1 && !shortcut;
  const text = e.key === "Enter" ? "\r" : types ? e.key : undefined;
  return {
    kind: "key",
    action,
    key: e.key,
    code: e.code,
    key_code: keyCode(e.key, e.code),
    modifiers: modifierMask(e),
    ...(action === "down" && text !== undefined ? { text } : {}),
  };
}

/** The mouse button DevTools names for a DOM button number. */
export function mouseButton(button: number): "left" | "middle" | "right" {
  return button === 1 ? "middle" : button === 2 ? "right" : "left";
}

/** How many pixels a wheel step of `mode` (DOM `deltaMode`) scrolls. */
export function wheelScale(mode: number, pageHeight: number): number {
  return mode === 1 ? 16 : mode === 2 ? pageHeight : 1;
}
