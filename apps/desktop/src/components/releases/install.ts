// Installing an iOS package on a paired iPhone or iPad (H-229, UX-043):
// when the Install box shows, and the words it uses.

import type { InstallDevice, Release, ReleaseInstall } from "../../protocol/releases";

/** The states that offer install actions (UX-043 §1). */
const OFFERED: ReadonlySet<string> = new Set([
  "awaiting_owner",
  "approved",
  "deploying",
  "deployed",
]);

/** Before submit: an iOS build not published yet says so. */
const BEFORE: ReadonlySet<string> = new Set(["planned", "assembling", "built"]);

/**
 * What the box shows for a package: nothing (no iOS build, or a state with
 * no install), the not-published sentence, or the install actions.
 */
export function installStage(release: Release): "hidden" | "unpublished" | "offered" {
  const ios = release.builds.find((b) => b.platform === "ios");
  if (!ios) {
    return "hidden";
  }
  if (!ios.install_url) {
    return OFFERED.has(release.status) || BEFORE.has(release.status) ? "unpublished" : "hidden";
  }
  return OFFERED.has(release.status) ? "offered" : "hidden";
}

const STATE_WORDS: Readonly<Record<string, string>> = {
  awaiting_owner: "ready for you to test",
  approved: "approved",
  deploying: "rolling out",
  deployed: "live",
};

/** "The Hermes 0.6.1 (12) · approved". */
export function installLine(release: Release, info: ReleaseInstall | null): string {
  const title = info?.app_title ?? "The Hermes";
  const version = info?.version ?? release.display_version ?? release.name;
  const build = info?.build ?? release.builds.find((b) => b.platform === "ios")?.version;
  const named = build && build !== version ? `${version} (${build})` : version;
  return `${title} ${named} · ${STATE_WORDS[release.status] ?? release.status}`;
}

const HOUR_MS = 3_600_000;

/** "seen 2h ago", "seen just now", "seen 3d ago", "never seen". */
function seen(device: InstallDevice, now: number): string {
  if (device.connected) {
    return "connected";
  }
  const then = device.last_seen_at ? Date.parse(device.last_seen_at) : Number.NaN;
  if (!Number.isFinite(then)) {
    return "never seen";
  }
  const hours = Math.max(0, (now - then) / HOUR_MS);
  if (hours < 1) {
    return "seen just now";
  }
  return hours < 24 ? `seen ${Math.round(hours)}h ago` : `seen ${Math.floor(hours / 24)}d ago`;
}

/** One device in Send's list: "iPad · seen 2h ago", or "iPhone 16 · has 0.6.1". */
export function deviceLine(device: InstallDevice, version: string, now: number): string {
  const name = device.name ?? "Device";
  if (device.app_version && version && device.app_version.startsWith(version)) {
    return `${name} · has ${version}`;
  }
  return `${name} · ${seen(device, now)}`;
}
