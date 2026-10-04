import { describe, expect, it } from "vitest";
import { keyInput, pagePoint } from "./browserInput";

const NONE = { altKey: false, ctrlKey: false, metaKey: false, shiftKey: false };

describe("pagePoint", () => {
  // An 800×600 page shown in a 1000×300 box: scaled by half, centered.
  const box = { left: 10, top: 20, width: 1000, height: 300 };
  const page = { width: 800, height: 600 };

  it("maps the shown screen back to the page", () => {
    expect(pagePoint(box, page, 310, 20)).toEqual({ x: 0, y: 0 });
    expect(pagePoint(box, page, 510, 170)).toEqual({ x: 400, y: 300 });
  });

  it("is null beside the page", () => {
    expect(pagePoint(box, page, 100, 100)).toBeNull();
    expect(pagePoint(box, { width: 0, height: 0 }, 310, 20)).toBeNull();
  });
});

describe("keyInput", () => {
  it("types printable keys and Enter", () => {
    expect(keyInput({ ...NONE, key: "a", code: "KeyA" }, "down")).toEqual({
      kind: "key",
      action: "down",
      key: "a",
      code: "KeyA",
      key_code: 65,
      modifiers: 0,
      text: "a",
    });
    expect(keyInput({ ...NONE, key: "Enter", code: "Enter" }, "down")).toMatchObject({
      key_code: 13,
      text: "\r",
    });
  });

  it("sends keys that type nothing by code, and shortcuts without text", () => {
    expect(keyInput({ ...NONE, key: "Backspace", code: "Backspace" }, "down")).not.toHaveProperty(
      "text",
    );
    expect(keyInput({ ...NONE, metaKey: true, key: "a", code: "KeyA" }, "down")).toMatchObject({
      modifiers: 4,
      key_code: 65,
    });
    expect(keyInput({ ...NONE, metaKey: true, key: "a", code: "KeyA" }, "down")).not.toHaveProperty(
      "text",
    );
    expect(keyInput({ ...NONE, key: "a", code: "KeyA" }, "up")).not.toHaveProperty("text");
  });

  it("leaves composing keys to the character they make", () => {
    expect(keyInput({ ...NONE, key: "Dead", code: "KeyE" }, "down")).toBeNull();
  });
});
