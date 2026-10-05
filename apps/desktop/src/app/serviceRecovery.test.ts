import { describe, expect, it } from "vitest";
import type { ServiceState, ServiceStatus } from "../setup";
import { failureHeadline, progressLabel, recoveryFor } from "./serviceRecovery";

function status(state: ServiceState, version: string | null = null): ServiceStatus {
  return { state, port: 49777, version };
}

describe("recoveryFor", () => {
  it("offers nothing for a healthy service", () => {
    expect(recoveryFor(status("healthy", "0.15.1"))).toBeNull();
  });

  it.each([
    ["not_installed", "Install the Hermes service"],
    ["legacy_only", "Finish the update"],
    ["migration_pending", "Finish the update"],
    ["migrated_service_missing", "Finish the update"],
    ["unmanaged", "Repair the service"],
    ["broken", "Repair the service"],
  ] as const)("offers one action for %s", (state, action) => {
    expect(recoveryFor(status(state))?.action).toBe(action);
  });

  it("names the Hermes started by hand without saying daemon", () => {
    const offer = recoveryFor(status("unmanaged", "0.14.2"));
    expect(offer?.title).toBe("Hermes is running, but not as a service");
    expect(offer?.body).toMatch(
      /^Hermes 0\.14\.2 is answering on port 49777, but it was started by hand/,
    );
    expect(offer?.body).toMatch(/press Ctrl-C in its Terminal window/);
    expect(offer?.body).toMatch(/Your bots pause until the service starts them again\.$/);
    expect(recoveryFor(status("unmanaged"))?.body).toMatch(/^Hermes is answering on port 49777/);
  });

  it.each([
    "not_installed",
    "legacy_only",
    "migration_pending",
    "migrated_service_missing",
    "unmanaged",
    "broken",
  ] as const)("keeps daemon out of the %s copy", (state) => {
    const offer = recoveryFor(status(state, "0.14.2"));
    expect(`${offer?.title ?? ""} ${offer?.body ?? ""}`).not.toMatch(/daemon/i);
  });

  it("says the legacy-only update leaves data where it is", () => {
    expect(recoveryFor(status("legacy_only"))?.body).toBe(
      "Your bots are still set up to run on the Hermes service from before the update, and it may be stopped. Finishing the update installs the new service in its place. Your data stays where it is.",
    );
  });

  it.each([
    ["Install the Hermes service", "Installing…", "Couldn't install the Hermes service."],
    ["Finish the update", "Finishing the update…", "Couldn't finish the update."],
    ["Repair the service", "Repairing…", "Couldn't repair the service."],
  ] as const)("names %s while it runs and when it fails", (action, progress, failure) => {
    expect(progressLabel(action)).toBe(progress);
    expect(failureHeadline(action)).toBe(failure);
  });

  it("says a pending migration moves data only after showing what moves", () => {
    expect(recoveryFor(status("migration_pending"))?.body).toMatch(
      /you see what moves before anything happens/,
    );
  });
});
