import { describe, expect, it } from "vitest";
import { confirmHomeMigration, onHomeMigrationRequest } from "./homeMigration";
import type { HomeMigrationRequest } from "./homeMigration";

describe("confirmHomeMigration", () => {
  it("answers no when no dialog is there to ask", async () => {
    await expect(confirmHomeMigration("Moves ~/.gravity")).resolves.toBe(false);
  });

  it("hands the summary to the host and resolves with its answer", async () => {
    const shown: (HomeMigrationRequest | null)[] = [];
    const stop = onHomeMigrationRequest((request) => {
      shown.push(request);
    });
    const answer = confirmHomeMigration("Moves ~/.gravity");
    expect(shown[0]?.summary).toBe("Moves ~/.gravity");
    shown[0]?.answer(true);
    await expect(answer).resolves.toBe(true);
    // The dialog is dismissed once answered.
    expect(shown[1]).toBeNull();

    stop();
    await expect(confirmHomeMigration("again")).resolves.toBe(false);
  });
});
