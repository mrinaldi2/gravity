import { describe, expect, it } from "vitest";
import { prDiff } from "../../test/prFixtures";
import { parseDiff } from "./diffParse";

describe("parseDiff", () => {
  it("splits files and numbers each line on its side", () => {
    const files = parseDiff(prDiff().diff);
    expect(files.map((f) => f.path)).toEqual([
      "apps/desktop/src/components/releases/ReleaseReview.tsx",
      "docs/user/releases.md",
    ]);
    const lines = files[0]?.lines ?? [];
    expect(lines.map((l) => [l.kind, l.number])).toEqual([
      ["hunk", null],
      ["ctx", 48],
      ["del", 49],
      ["add", 49],
      ["add", 50],
      ["ctx", 51],
      ["ctx", 52],
    ]);
    expect(lines[3]?.text).toContain("const approve");
  });

  it("notes binaries and a missing newline, and ignores text before the first file", () => {
    const files = parseDiff(
      [
        "garbage",
        "diff --git a/img.png b/img.png",
        "Binary files a/img.png and b/img.png differ",
        "diff --git a/x.txt b/x.txt",
        "@@ -1 +1 @@",
        "-a",
        "+b",
        "\\ No newline at end of file",
      ].join("\n"),
    );
    expect(files.map((f) => [f.path, f.lines.map((l) => l.kind)])).toEqual([
      ["img.png", ["note"]],
      ["x.txt", ["hunk", "del", "add", "note"]],
    ]);
    expect(files[1]?.lines[3]?.text).toBe("No newline at end of file");
  });
});
