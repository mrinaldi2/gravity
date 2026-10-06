// How far a release is (H-137, owner ruling 91890778): a readiness line and
// each item's live status, shown until the package is submitted, so the
// owner sees when it will be ready.

import type { ReactElement } from "react";
import type { PlanItem, Release } from "../../protocol/releases";
import type { BotName } from "./labels";

/** Shown for a package not yet submitted that carries its items' status. */
export function showsProgress(release: Release): boolean {
  return (
    ["planned", "assembling", "built"].includes(release.status) && (release.plan?.length ?? 0) > 0
  );
}

/** "3 of 5 items ready · Built for desktop-mac · Tested on 1 of 2 computers". */
export function readinessLine(release: Release): string {
  const r = release.readiness;
  if (!r) {
    return "";
  }
  const builds = r.builds.length ? `Built for ${r.builds.join(", ")}` : "Not built yet";
  const tests = r.tests_required.length
    ? `Tested on ${r.tests_passed.length} of ${r.tests_required.length} computers`
    : "No computer to test on yet";
  return `${r.items_ready} of ${r.items_total} items ready · ${builds} · ${tests}`;
}

/** A column key as a word: "doing" → "Doing". */
function columnWord(key: string): string {
  return key.charAt(0).toUpperCase() + key.slice(1).replace(/_/gu, " ");
}

function ItemRow({ item, botName }: { item: PlanItem; botName: BotName }): ReactElement {
  const who = item.assignee ? (botName(item.assignee) ?? "a bot") : "nobody yet";
  return (
    <li className="release-progress-row">
      <span aria-hidden="true">{item.ready ? "✓" : "○"}</span>
      <span className="mono">{item.item_id}</span>
      <span className="release-progress-title">{item.title}</span>
      <span className="release-meta">
        {item.ready ? "Ready" : "Not ready"} · {columnWord(item.column_key)} · {who}
        {item.ac_total ? ` · criteria ${item.ac_checked}/${item.ac_total}` : ""}
        {item.blocked ? " · ⛔ Blocked" : ""}
      </span>
    </li>
  );
}

export default function ReleaseProgress({
  release,
  botName,
}: {
  readonly release: Release;
  readonly botName: BotName;
}): ReactElement | null {
  if (!showsProgress(release)) {
    return null;
  }
  return (
    <section className="release-progress" aria-label="Progress">
      <h3>Progress</h3>
      <p className="release-meta">{readinessLine(release)}</p>
      <ul>
        {(release.plan ?? []).map((item) => (
          <ItemRow key={item.item_id} item={item} botName={botName} />
        ))}
      </ul>
    </section>
  );
}
