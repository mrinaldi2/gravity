// Words Needs you and its relayed-rulings dialog share (H-112).

export function when(at: string): string {
  return new Date(at).toLocaleString([], {
    weekday: "short",
    hour: "2-digit",
    minute: "2-digit",
  });
}

/** "Team Lead", "Team Lead and PM", "Team Lead, PM and QA". */
export function names(list: readonly string[]): string {
  if (list.length <= 1) {
    return list[0] ?? "A bot";
  }
  return `${list.slice(0, -1).join(", ")} and ${list.at(-1) ?? ""}`;
}
