import { describe, expect, it } from "vitest";
import { readStored, removeStored, writeStored } from "./storage";
import { stubLocalStorage } from "./test/spies";

describe("storage", () => {
  it("reads a value saved before the rename", () => {
    stubLocalStorage({ "gravity.connection": "old" });
    expect(readStored("connection")).toBe("old");
  });

  it("prefers the new key and writes only the new key", () => {
    const store = stubLocalStorage({ "gravity.connection": "old" });
    writeStored("connection", "new");
    expect(readStored("connection")).toBe("new");
    expect(store.get("hermes.connection")).toBe("new");
    expect(store.get("gravity.connection")).toBe("old");
  });

  it("removes both spellings so the old value does not come back", () => {
    stubLocalStorage({ "gravity.device-token": "old" });
    writeStored("device-token", "new");
    removeStored("device-token");
    expect(readStored("device-token")).toBeNull();
  });
});
