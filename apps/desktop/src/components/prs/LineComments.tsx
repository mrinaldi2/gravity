// Line comments in a PR's diff (UX-051 "files", decision 7; H-282): each
// thread under its line, with its replies, Reply and Resolve; and the box to
// start one. A thread whose line is gone on the head is listed as outdated.
// The owner's comments and resolves go over the app's own connection.

import { useEffect, useRef, useState } from "react";
import type { ReactElement } from "react";
import type { LineComment } from "../../protocol/gen/hermes/pr/v1/pr_pb";
import { Severity, Side } from "../../protocol/gen/hermes/pr/v1/pr_pb";
import type { PrOwnerRequestBody } from "../../protocol/prOwner";

export interface Thread {
  readonly root: LineComment;
  readonly replies: readonly LineComment[];
}

/** Comments in threads: each first comment with the replies to it, in order. */
export function threadsOf(comments: readonly LineComment[]): Thread[] {
  const roots = comments.filter((c) => c.replyTo === "");
  return roots.map((root) => ({
    root,
    replies: comments.filter((c) => c.replyTo === root.id),
  }));
}

/** The diff line a thread sits under: its side and its number there. */
export function lineKey(path: string, side: Side, line: number): string {
  return `${path}:${side === Side.OLD ? "old" : "new"}:${line}`;
}

const SEVERITY: Readonly<Record<Severity, string>> = {
  [Severity.UNSPECIFIED]: "",
  [Severity.MUST]: "Must-fix",
  [Severity.SHOULD]: "Should-fix",
  [Severity.NIT]: "Nit",
};

function authorName(c: LineComment): string {
  const who = c.author?.who;
  if (who?.case === "owner") {
    return "You";
  }
  return who?.case === "bot" ? who.value.name : "A reviewer";
}

/** What a comment write sends, before the PR's own fields are added. */
export type CommentWrite =
  | Omit<Extract<PrOwnerRequestBody, { type: "pr_comment_add" }>, "project_id" | "number" | "sha">
  | Omit<Extract<PrOwnerRequestBody, { type: "pr_comment_resolve" }>, "project_id" | "number">;

/** A text box with Comment and Cancel; Ctrl/⌘-Enter sends. */
export function Composer(props: {
  readonly label: string;
  readonly busy: boolean;
  /** Offers a severity: a new thread can be a must-fix, a reply can't. */
  readonly severity?: boolean;
  readonly onSend: (body: string, severity: "" | "must" | "should" | "nit") => Promise<boolean>;
  readonly onCancel: () => void;
}): ReactElement {
  const [body, setBody] = useState("");
  const [severity, setSeverity] = useState<"" | "must" | "should" | "nit">("");
  const box = useRef<HTMLTextAreaElement>(null);
  useEffect(() => {
    box.current?.focus();
  }, []);
  const send = async (): Promise<void> => {
    if (body.trim() !== "" && (await props.onSend(body.trim(), severity))) {
      setBody("");
    }
  };
  return (
    <div className="pr-composer">
      <textarea
        ref={box}
        rows={2}
        aria-label={props.label}
        placeholder={props.label}
        value={body}
        onChange={(event) => setBody(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) {
            event.preventDefault();
            void send();
          } else if (event.key === "Escape") {
            event.stopPropagation();
            props.onCancel();
          }
        }}
      />
      <div className="pr-composer-actions">
        {props.severity ? (
          <select
            aria-label="Kind of comment"
            value={severity}
            onChange={(event) => setSeverity(event.target.value as typeof severity)}
          >
            <option value="">Comment</option>
            <option value="must">Must-fix</option>
            <option value="should">Should-fix</option>
            <option value="nit">Nit</option>
          </select>
        ) : null}
        <button type="button" className="btn btn-small" onClick={props.onCancel}>
          Cancel
        </button>
        <button
          type="button"
          className="btn btn-small btn-primary"
          disabled={props.busy || body.trim() === ""}
          onClick={() => void send()}
        >
          Comment
        </button>
      </div>
    </div>
  );
}

function CommentBody({ c }: { readonly c: LineComment }): ReactElement {
  const severity = SEVERITY[c.severity];
  return (
    <div className="pr-comment">
      <b>{authorName(c)}</b>
      {severity ? <span className="pr-dim"> · {severity}</span> : null}
      <div className="pr-comment-text">{c.body}</div>
    </div>
  );
}

/** One thread: the first comment, its replies, then Reply and Resolve (owner only). */
export function CommentThread(props: {
  readonly thread: Thread;
  readonly canWrite: boolean;
  readonly busy: boolean;
  readonly onWrite: (write: CommentWrite) => Promise<boolean>;
}): ReactElement {
  const { root, replies } = props.thread;
  const [replying, setReplying] = useState(false);
  const where = `${root.path}:${root.line}`;
  if (root.resolved) {
    return (
      <details className="pr-thread pr-thread-resolved">
        <summary>
          ✓ Resolved · {authorName(root)} on line {root.line}
        </summary>
        <CommentBody c={root} />
        {replies.map((reply) => (
          <CommentBody key={reply.id} c={reply} />
        ))}
      </details>
    );
  }
  return (
    <div className="pr-thread" aria-label={`Comments on ${where}`} role="group">
      <CommentBody c={root} />
      {replies.map((reply) => (
        <CommentBody key={reply.id} c={reply} />
      ))}
      {replying ? (
        <Composer
          label={`Reply on ${where}`}
          busy={props.busy}
          onCancel={() => setReplying(false)}
          onSend={async (body) => {
            const done = await props.onWrite({
              type: "pr_comment_add",
              body,
              reply_to: root.id,
            });
            if (done) {
              setReplying(false);
            }
            return done;
          }}
        />
      ) : props.canWrite ? (
        <div className="pr-thread-actions">
          <button type="button" className="btn btn-small" onClick={() => setReplying(true)}>
            Reply
          </button>
          <button
            type="button"
            className="btn btn-small"
            disabled={props.busy}
            onClick={() => void props.onWrite({ type: "pr_comment_resolve", comment_id: root.id })}
          >
            Resolve
          </button>
        </div>
      ) : null}
    </div>
  );
}
