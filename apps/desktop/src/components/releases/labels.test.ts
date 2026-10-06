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
});
