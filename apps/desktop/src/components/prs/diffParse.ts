// A unified diff (`pr_diff`'s text, git's format) split into files and lines
// with their numbers, for a read-only view.

type DiffLineKind = "add" | "del" | "ctx" | "hunk" | "note";

interface DiffLine {
  readonly kind: DiffLineKind;
  /** The line's number on its side: the old file for a deletion, the new one otherwise. */
  readonly number: number | null;
  readonly text: string;
}

export interface DiffFileText {
  readonly path: string;
  readonly lines: readonly DiffLine[];
}

const HUNK = /^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@/;

/** "diff --git a/x b/y" → "y". */
function gitPath(line: string): string {
  const at = line.lastIndexOf(" b/");
  return at < 0 ? line.slice("diff --git ".length) : line.slice(at + 3);
}

interface FileState {
  path: string;
  readonly lines: DiffLine[];
  /** The next line's numbers on each side; null before the first hunk. */
  at: { old: number; new: number } | null;
}

/** A header line, before the file's first hunk: the new path, or a binary file. */
function header(file: FileState, line: string): void {
  if (line.startsWith("+++ b/")) {
    file.path = line.slice("+++ b/".length);
  } else if (line.startsWith("Binary files")) {
    file.lines.push({ kind: "note", number: null, text: "Binary file, not shown" });
  }
}

/** A line inside a hunk, numbered on its side. */
function body(file: FileState, at: { old: number; new: number }, line: string): void {
  const text = line.slice(1);
  switch (line.charAt(0)) {
    case "+":
      file.lines.push({ kind: "add", number: at.new, text });
      at.new += 1;
      break;
    case "-":
      file.lines.push({ kind: "del", number: at.old, text });
      at.old += 1;
      break;
    case " ":
      file.lines.push({ kind: "ctx", number: at.new, text });
      at.old += 1;
      at.new += 1;
      break;
    case "\\":
      file.lines.push({ kind: "note", number: null, text: line.slice(2) });
      break;
    default:
      break;
  }
}

export function parseDiff(text: string): DiffFileText[] {
  const files: FileState[] = [];
  for (const line of text.split("\n")) {
    const current = files.at(-1);
    const hunk = HUNK.exec(line);
    if (line.startsWith("diff --git ")) {
      files.push({ path: gitPath(line), lines: [], at: null });
    } else if (current === undefined) {
      continue;
    } else if (hunk !== null) {
      current.at = { old: Number(hunk[1]), new: Number(hunk[2]) };
      current.lines.push({ kind: "hunk", number: null, text: line });
    } else if (current.at === null) {
      header(current, line);
    } else {
      body(current, current.at, line);
    }
  }
  return files.map(({ path, lines }) => ({ path, lines }));
}
