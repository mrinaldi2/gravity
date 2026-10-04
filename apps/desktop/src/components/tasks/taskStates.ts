import type { BotTask } from "../../protocol/tasks";

/** Task states in words (ux-glossary §4). */
export const TASK_STATE_LABEL: Readonly<Record<BotTask["state"], string>> = {
  open: "Open",
  done: "Done",
  cancelled: "Cancelled",
  expired: "Expired",
};

/** A task state from the wire in words; a state this build doesn't know reads "Unknown". */
export function taskStateLabel(state: string): string {
  const known = Object.entries(TASK_STATE_LABEL).find(([key]) => key === state);
  return known === undefined ? "Unknown" : known[1];
}
