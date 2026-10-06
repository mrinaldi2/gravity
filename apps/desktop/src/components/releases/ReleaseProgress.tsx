// How far a release is (H-137, owner ruling 91890778; copy from UX-025): a
// sentence on what is left, a readiness line, and each item's live status,
// shown until the package is submitted, so the owner sees when it will be
// ready.

import type { ReactElement } from "react";
import type { PlanItem, Release } from "../../protocol/releases";
import type { BotName } from "./labels";

/** How many unready items the "what's left" sentence names. */
const NAMED = 3;

/** The owner's words for a build platform. */
const PLATFORM: Readonly<Record<string, string>> = {
  "desktop-mac": "Mac",
  "desktop-win": "Windows",
  ios: "iPhone",
  daemon: "the service",
};

function platformName(platform: string): string {
  return PLATFORM[platform] ?? platform;
}

/** Shown for a package not yet submitted that carries its items' status. */
export function showsProgress(release: Release): boolean {
  return (
    ["planned", "assembling", "built"].includes(release.status) && (release.plan?.length ?? 0) > 0
  );
}

/** "H-1, H-2, H-3 and 2 more". */
function named(ids: readonly string[]): string {
  const shown = ids.slice(0, NAMED).join(", ");
  const rest = ids.length - NAMED;
  return rest > 0 ? `${shown} and ${rest} more` : shown;
}

/** What is left before the release is ready, in one sentence per state. */
export function leftSentence(release: Release): string {
  const r = release.readiness;
  const waiting = (release.plan ?? []).filter((p) => !p.ready).map((p) => p.item_id);
  if (release.status === "planned") {
    return waiting.length
      ? `Waiting on ${waiting.length} of ${r?.items_total ?? waiting.length} items: ${named(waiting)}. It is built once every item is ready.`
      : "Every item is ready. Next, DevOps packages and builds it.";
  }
  if (release.status === "assembling") {
    return "Every item is ready. Next, DevOps attaches the builds.";
  }
  const untested = (r?.tests_required ?? []).filter((m) => !(r?.tests_passed ?? []).includes(m));
  return untested.length
    ? `Built. Waiting for tests on ${named(untested)}.`
    : "Built and tested. Next, DevOps sends it to you.";
}

/** "1 of 3 items ready · Built for Mac · Tested on 0 of 2 computers". */
export function readinessLine(release: Release): string {
  const r = release.readiness;
  if (!r) {
    return "";
  }
  const builds = r.builds.length
    ? `Built for ${r.builds.map(platformName).join(", ")}`
    : "Not built yet";
  const tests = r.tests_required.length
    ? `Tested on ${r.tests_passed.length} of ${r.tests_required.length} computers`
    : "No computer to test on yet";
  return `${r.items_ready} of ${r.items_total} items ready · ${builds} · ${tests}`;
}

function ItemRow({ item, botName }: { item: PlanItem; botName: BotName }): ReactElement {
  const who = item.assignee ? (botName(item.assignee) ?? "A bot") : "Unassigned";
  return (
    <li className="release-progress-row">
      <span>{item.ready ? "✓ Ready for the release" : "○ Still in progress"}</span>
      <span className="mono">{item.item_id}</span>
      <span className="release-progress-title">{item.title}</span>
      <span className="release-meta">
        {item.column_name ?? item.column_key} · {who}
        {item.ac_total ? ` · ☑ ${item.ac_checked}/${item.ac_total} AC` : ""}
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
      <p>{leftSentence(release)}</p>
      <p className="release-meta">{readinessLine(release)}</p>
      <ul>
        {(release.plan ?? []).map((item) => (
          <ItemRow key={item.item_id} item={item} botName={botName} />
        ))}
      </ul>
    </section>
  );
}
