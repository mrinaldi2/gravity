import { describe, expect, it } from "vitest";
import type { ServiceState, ServiceStatus } from "../setup";
import { recoveryFor } from "./serviceRecovery";

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

  it("names the daemon running outside the service", () => {
    expect(recoveryFor(status("unmanaged", "0.14.2"))?.body).toMatch(
      /^Hermes 0\.14\.2 answers on port 49777, but no installed service runs it/,
    );
    expect(recoveryFor(status("unmanaged"))?.body).toMatch(/^A Hermes daemon answers/);
  });

  it("says a pending migration moves data only after showing what moves", () => {
    expect(recoveryFor(status("migration_pending"))?.body).toMatch(
      /you see what moves before anything happens/,
    );
  });
});
