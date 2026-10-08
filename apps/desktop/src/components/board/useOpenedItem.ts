import { useState } from "react";

/**
 * The card whose drawer is open on the board. A new "Open on the board"
 * (UX-035 §4) opens its card even when the board is already showing.
 */
export function useOpenedItem(
  asked: string | undefined,
): [string | null, (id: string | null) => void] {
  const [opened, setOpened] = useState<string | null>(asked ?? null);
  const [last, setLast] = useState(asked);
  if (asked !== last) {
    setLast(asked);
    setOpened(asked ?? opened);
  }
  return [opened, setOpened];
}
