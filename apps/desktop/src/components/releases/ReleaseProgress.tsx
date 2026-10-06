// How far a release is (H-137, owner ruling 91890778; copy from
// artifacts/UX-025-H-137.md): a readiness line, a sentence on what is left,
// and each item's live status, shown until the package is submitted, so the
// owner sees when it will be ready.

import type { ReactElement } from "react";
import type { PlanItem, Release } from "../../protocol/releases";
import { plural } from "./labels";
import type { BotName } from "./labels";

/** How many unready items the "what's left" sentence names. */
const NAMED = 3;

/** The owner's words for a build platform (UX-025 §4). */
const PLATFORM: Readonly<Record<string, string>> = {
  "desktop-mac": "Mac",
  "desktop-win": "Windows",
  "desktop-linux": "Linux",
  ios: "iPhone",
};

/** Column names when the board doesn't give one (UX-025 §2). */
const COLUMN: Readonly<Record<string, string>> = {
  inbox: "Inbox",
  ready: "Ready",
  doing: "Doing",
  review: "Review",
  verify: "Verify",
  approval: "Awaiting owner",
  deploying: "Deploying",
  done: "Done",
  cancelled: "Cancelled",
};

function platformName(platform: string): string {
  return PLATFORM[platform] ?? platform;
}

/** The board's name for the item's column, else the fallback map. */
function columnName(item: PlanItem): string {
  if (item.column_name) {
    return item.column_name;
  }
  const key = item.column_key;
  return COLUMN[key] ?? key.charAt(0).toUpperCase() + key.slice(1).replace(/_/gu, " ");
}

/** "A", "A and B", "A, B and C". */
function andList(words: readonly string[]): string {
  if (words.length < 2) {
    return words.join("");
  }
  return `${words.slice(0, -1).join(", ")} and ${words[words.length - 1]}`;
}

/** Shown for a package not yet submitted that carries its items' status. */
export function showsProgress(release: Release): boolean {
  return (
    ["planned", "assembling", "built"].includes(release.status) && (release.plan?.length ?? 0) > 0
  );
}

/** "1 of 3 items ready · Built for Mac and Windows · Tested on 0 of 2 computers". */
export function readinessLine(release: Release): string {
  const r = release.readiness;
  if (!r) {
    return "";
  }
  const builds = r.builds.length
    ? `Built for ${andList(r.builds.map(platformName))}`
    : "Not built yet";
  const tests = r.tests_required.length
    ? `Tested on ${r.tests_passed.length} of ${plural(r.tests_required.length, "computer")}`
    : "No computer to test on yet";
  return `${r.items_ready} of ${r.items_total} items ready · ${builds} · ${tests}`;
}

/** "H-117 in Doing, H-021 in Review, ⛔ blocked, +2 more". */
function notReadyList(waiting: readonly PlanItem[]): string {
  const named = waiting
    .slice(0, NAMED)
    .map((p) => `${p.item_id} in ${columnName(p)}${p.blocked ? ", ⛔ blocked" : ""}`);
  const more = waiting.length - NAMED;
  return more > 0 ? `${named.join(", ")}, +${more} more` : named.join(", ");
}

/** What is left before it is ready: the first rule of UX-025 §5 that matches. */
export function leftSentence(release: Release): string {
  const r = release.readiness;
  const required = plural(r?.tests_required.length ?? 0, "computer");
  if (release.status === "planned") {
    const waiting = (release.plan ?? []).filter((p) => !p.ready);
    return waiting.length
      ? `Packaging starts when every item reaches Verify: ${waiting.length} to go (${notReadyList(waiting)}).`
      : "Every item has reached Verify. DevOps can start packaging it now.";
  }
  if (release.status === "assembling") {
    const builds = r?.builds ?? [];
    return builds.length
      ? `DevOps is building it: built for ${andList(builds.map(platformName))} so far. Next: tests on ${required}.`
      : `DevOps is building it. Next: tests on ${required}.`;
  }
  const passed = r?.tests_passed.length ?? 0;
  return passed < (r?.tests_required.length ?? 0)
    ? `Being tested: ${passed} of ${required} passed. It comes to you to test when all of them pass.`
    : "Every computer passed. DevOps sends it to you to test next.";
}

function ItemRow({ item, botName }: { item: PlanItem; botName: BotName }): ReactElement {
  const who = item.assignee ? (botName(item.assignee) ?? "A bot") : "Unassigned";
  return (
    <li className="release-progress-row">
      <span>
        <span aria-hidden="true">{item.ready ? "✓" : "○"}</span>{" "}
        {item.ready ? "Ready for the release" : "Still in progress"}
      </span>
      <span className="mono">{item.item_id}</span>
      <span className="release-progress-title">{item.title}</span>
      <span className="release-meta">
        {columnName(item)} · {who}
        {item.ac_total ? (
          <>
            {" · "}
            <span aria-hidden="true">
              ☑ {item.ac_checked}/{item.ac_total} AC
            </span>
            <span className="visually-hidden">
              {item.ac_checked} of {item.ac_total} acceptance criteria
            </span>
          </>
        ) : null}
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
      <p>{leftSentence(release)}</p>
      <ul>
        {(release.plan ?? []).map((item) => (
          <ItemRow key={item.item_id} item={item} botName={botName} />
        ))}
      </ul>
    </section>
  );
}
