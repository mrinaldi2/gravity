// One file's unified diff with its line comments (UX-051 "files"): each
// thread sits under the line it is about, and "＋" on a line starts one.
// Threads whose line is gone on the head are listed after the file.

import { useState } from "react";
import type { ReactElement } from "react";
import { Side } from "../../protocol/gen/hermes/pr/v1/pr_pb";
import type { DiffFileText } from "./diffParse";
import type { CommentWrite, Thread } from "./LineComments";
import { CommentThread, Composer, lineKey } from "./LineComments";

const SIGNS = { add: "+", del: "−", ctx: " ", hunk: "", note: "" } as const;

/** Where a diff line can be commented: deletions on the old side, the rest on the new. */
function anchorOf(
  kind: keyof typeof SIGNS,
  number: number | null,
): { side: Side; line: number } | null {
  if (number === null || kind === "hunk" || kind === "note") {
    return null;
  }
  return { side: kind === "del" ? Side.OLD : Side.NEW, line: number };
}

export interface DiffComments {
  /** Threads by `lineKey`, as shown on the head. */
  readonly at: ReadonlyMap<string, readonly Thread[]>;
  readonly outdated: readonly Thread[];
  /** The owner can write: connected, with approve. */
  readonly canWrite: boolean;
  /** Old-side lines take comments only in the diff from main (H-282 M2). */
  readonly oldSide: boolean;
  readonly busy: boolean;
  readonly onWrite: (write: CommentWrite) => Promise<boolean>;
}

export default function FileDiff(props: {
  readonly file: DiffFileText;
  readonly range: string;
  readonly comments: DiffComments;
}): ReactElement {
  const { file, comments } = props;
  const [composing, setComposing] = useState<string | null>(null);
  const outdated = comments.outdated.filter((t) => t.root.path === file.path);
  const thread = (t: Thread): ReactElement => (
    <CommentThread
      key={t.root.id}
      thread={t}
      canWrite={comments.canWrite}
      busy={comments.busy}
      onWrite={comments.onWrite}
    />
  );
  return (
    <section className="pr-diff" aria-label={`Diff of ${file.path}`}>
      <div className="pr-diff-head">
        {file.path} · <span className="pr-sha">{props.range}</span>
      </div>
      {file.lines.map((line, index) => {
        const anchor = anchorOf(line.kind, line.number);
        const key = anchor === null ? null : lineKey(file.path, anchor.side, anchor.line);
        const open =
          anchor !== null && comments.canWrite && (anchor.side === Side.NEW || comments.oldSide);
        return (
          // Lines have no identity of their own; their place in the file is it.
          // oxlint-disable-next-line react/no-array-index-key
          <div key={index}>
            <div className={`pr-diff-line pr-diff-${line.kind}`}>
              <span className="pr-diff-number">
                {open && key !== null ? (
                  <button
                    type="button"
                    className="pr-diff-comment"
                    aria-label={`Comment on line ${line.number ?? ""}`}
                    onClick={() => setComposing(key)}
                  >
                    ＋
                  </button>
                ) : null}
                {line.number ?? ""}
              </span>
              <span className="pr-diff-sign">{SIGNS[line.kind]}</span>
              <span className="pr-diff-text">{line.text}</span>
            </div>
            {key === null ? null : (comments.at.get(key) ?? []).map(thread)}
            {key !== null && key === composing && anchor !== null ? (
              <Composer
                label={`Comment on line ${anchor.line}`}
                severity
                busy={comments.busy}
                onCancel={() => setComposing(null)}
                onSend={async (body, severity) => {
                  const done = await comments.onWrite({
                    type: "pr_comment_add",
                    path: file.path,
                    line: anchor.line,
                    side: anchor.side === Side.OLD ? "old" : "new",
                    body,
                    ...(severity === "" ? {} : { severity }),
                  });
                  if (done) {
                    setComposing(null);
                  }
                  return done;
                }}
              />
            ) : null}
          </div>
        );
      })}
      {outdated.length > 0 ? (
        <div className="pr-outdated">
          <div className="pr-dim">Outdated: their lines changed since</div>
          {outdated.map(thread)}
        </div>
      ) : null}
    </section>
  );
}
