import { describe, expect, it } from "vitest";
import { deployment, release } from "../../test/releaseFixtures";
import { eventLine, rolloutLabel } from "./labels";

describe("rolloutLabel", () => {
  it("shows an install called off by a later package as replaced, not installing", () => {
    const closed = release({
      status: "deployed",
      deployments: [
        deployment({ machine: "mac", result: "ok" }),
        deployment({ machine: "imac", result: "superseded" }),
      ],
    });
    expect(rolloutLabel(closed, "imac")).toEqual({ glyph: "⊘", word: "Replaced", tone: "off" });
    expect(rolloutLabel(closed, "mac").word).toBe("Live");
  });
});

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

  it("says a later deploy superseded a package nobody ruled on, or why it stayed open", () => {
    const base = {
      release_id: "rel-1",
      release_name: "0.17.0-r1",
      related_id: "rel-3",
      actor: "tester",
      at: "2026-10-08T08:00:00Z",
    };
    expect(
      eventLine(
        { ...base, kind: "superseded", note: "superseded by 0.17.2, deployed" },
        () => "Tester",
      ),
    ).toBe("0.17.0-r1 was superseded by 0.17.2, deployed, so it no longer needs your ruling.");
    expect(
      eventLine(
        { ...base, kind: "not_closed", note: "0.17.2 is deployed but didn't close this one: x." },
        () => "Tester",
      ),
    ).toBe("0.17.0-r1 stayed open: 0.17.2 is deployed but didn't close this one: x.");
  });

  it("says the daemon settled an iOS package installed on iOS", () => {
    const line = eventLine(
      {
        release_id: "rel-6",
        release_name: "iOS 0.6.1",
        related_id: null,
        kind: "targets_settled",
        actor: "daemon",
        note: "its deploy on ios counts for the iPhone (H-231)",
        at: "2026-10-08T08:00:00Z",
      },
      () => undefined,
    );
    expect(line).toBe(
      "Hermes marked iOS 0.6.1 deployed: its install on iOS counts for the iPhone.",
    );
  });
});
