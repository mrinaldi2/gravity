import { describe, expect, it } from "vitest";
import { eventLine } from "./labels";

describe("eventLine", () => {
  it("says a package was closed through the later release that contains it", () => {
    const line = eventLine(
      {
        release_id: "rel-2",
        release_name: "0.16.2",
        related_id: "rel-4",
        kind: "deployed_via",
        actor: "ops",
        note: "deployed via 0.16.4",
        at: "2026-10-06T08:00:00Z",
      },
      (id) => (id === "ops" ? "DevOps" : undefined),
    );
    expect(line).toBe("DevOps closed 0.16.2: deployed via 0.16.4.");
  });

  it("says the daemon moved an iOS package's deploy to the iPhone", () => {
    const line = eventLine(
      {
        release_id: "rel-5",
        release_name: "iOS 0.5.0",
        related_id: null,
        kind: "targets_repaired",
        actor: "daemon",
        note: "frozen before per-platform targets",
        at: "2026-10-07T08:00:00Z",
      },
      () => undefined,
    );
    expect(line).toBe(
      "Hermes moved iOS 0.5.0's deploy to the iPhone: frozen before per-platform targets.",
    );
  });
});
