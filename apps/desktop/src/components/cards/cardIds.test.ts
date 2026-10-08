import { describe, expect, it } from "vitest";
import { cardIdPattern, isCardId, splitCardIds } from "./cardIds";

const ids = (text: string, prefixes = ["H"]): string[] =>
  splitCardIds(text, cardIdPattern(prefixes))
    .filter((s) => s.kind === "id")
    .map((s) => (s.kind === "id" ? s.id : ""));

describe("card ids", () => {
  it("links only known prefixes, standing as a word (UX-035 test 1)", () => {
    expect(ids("see H-189 and UTF-8, branch H-189-project-clicks, `H-190`")).toEqual([
      "H-189",
      "H-190",
    ]);
  });

  it("keeps the text around the ids", () => {
    const parts = splitCardIds("H-1, then H-22.", cardIdPattern(["H"]));
    expect(parts).toEqual([
      { kind: "id", id: "H-1" },
      { kind: "text", text: ", then " },
      { kind: "id", id: "H-22" },
      { kind: "text", text: "." },
    ]);
  });

  it("leaves words that only look like ids", () => {
    expect(ids("XH-12 H-123456 H-12a sub-H-3 H-7_x")).toEqual([]);
    expect(ids("SHA-256", ["H"])).toEqual([]);
    expect(ids("SHA-256", ["SHA"])).toEqual(["SHA-256"]);
  });

  it("prefers the longer of two prefixes", () => {
    expect(ids("HX-4 and H-5", ["H", "HX"])).toEqual(["HX-4", "H-5"]);
  });

  it("knows nothing without a prefix", () => {
    expect(ids("H-1", [])).toEqual([]);
    expect(isCardId("H-17", cardIdPattern(["H"]))).toBe(true);
    expect(isCardId("see H-17", cardIdPattern(["H"]))).toBe(false);
  });
});
