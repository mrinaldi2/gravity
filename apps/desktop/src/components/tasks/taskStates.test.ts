import { describe, expect, it } from "vitest";
import { taskStateLabel } from "./taskStates";

describe("taskStateLabel", () => {
  it("puts a wire state into words", () => {
    expect(taskStateLabel("open")).toBe("Open");
    expect(taskStateLabel("cancelled")).toBe("Cancelled");
    expect(taskStateLabel("snoozed")).toBe("Unknown");
  });
});
