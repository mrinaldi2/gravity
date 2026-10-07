// The item drawer's Activity tab (H-018 §3.6): comments and history as one
// timeline, replies nested under their comment, and the owner's composer.
// A comment shows at once as "Sending…", then "✓ Posted"; a refused one
// keeps its text with Retry (H-201).

import { useEffect, useState } from "react";
import type { KeyboardEvent, ReactElement } from "react";
import type { ItemComment, ItemEvent } from "../../../protocol/gen/hermes/board/v1/board_pb";
import { ItemEventKind } from "../../../protocol/gen/hermes/board/v1/board_pb";
import type { ItemDetail } from "../../../protocol/gen/hermes/board/v1/requests_pb";
import { eventLine, when } from "./drawerText";

/** How long "✓ Posted" stays. */
const POSTED_MS = 3000;

type Filter = "all" | "comments" | "moves";

type Entry =
  | { readonly kind: "comment"; readonly at: number; readonly comment: ItemComment }
  | { readonly kind: "event"; readonly at: number; readonly event: ItemEvent };

/** Posts a comment, a reply when `replyTo` is set; rejects with the refusal. */
export type PostComment = (body: string, replyTo?: string) => Promise<void>;

function stamp(at: { readonly seconds: bigint } | undefined): number {
  return at === undefined ? 0 : Number(at.seconds);
}

/** Top-level entries, oldest first; a reply to an unknown comment stays top-level. */
function entriesOf(detail: ItemDetail, filter: Filter): Entry[] {
  const ids = new Set(detail.comments.map((c) => c.id));
  const entries: Entry[] = [
    ...detail.comments
      .filter((c) => c.replyTo === undefined || !ids.has(c.replyTo))
      .map((c) => ({ kind: "comment" as const, at: stamp(c.at), comment: c })),
    ...detail.history
      .filter((e) => e.kind !== ItemEventKind.COMMENTED)
      .map((e) => ({ kind: "event" as const, at: stamp(e.at), event: e })),
  ];
  // Newest last, as a conversation reads.
  // oxlint-disable-next-line unicorn/no-array-sort
  entries.sort((a, b) => a.at - b.at);
  return entries.filter(
    (e) =>
      filter === "all" ||
      (filter === "comments" && e.kind === "comment") ||
      (filter === "moves" && e.kind === "event" && e.event.kind === ItemEventKind.MOVED),
  );
}

function CommentLine(props: {
  readonly comment: ItemComment;
  readonly all: readonly ItemComment[];
  readonly who: (actor: string) => string;
  readonly onReply?: (comment: ItemComment) => void;
}): ReactElement {
  const { comment, who, onReply } = props;
  const replies = props.all.filter((c) => c.replyTo === comment.id);
  return (
    <li className={who(comment.author) === "You" ? "drawer-owner" : ""}>
      <span className="drawer-dim">{when(comment.at)}</span> <b>{who(comment.author)}</b>:{" "}
      {comment.body}
      {onReply === undefined ? null : (
        <button
          type="button"
          className="drawer-reply"
          aria-label={`Reply to ${who(comment.author)}`}
          onClick={() => onReply(comment)}
        >
          Reply
        </button>
      )}
      {replies.length === 0 ? null : (
        <ol className="drawer-replies" aria-label="Replies">
          {replies.map((r) => (
            <CommentLine key={r.id} comment={r} all={props.all} who={who} onReply={onReply} />
          ))}
        </ol>
      )}
    </li>
  );
}

function Composer(props: {
  readonly itemId: string;
  readonly replyTo: ItemComment | null;
  readonly who: (actor: string) => string;
  readonly onCancelReply: () => void;
  readonly onComment: PostComment;
  readonly onSending: (body: string | null) => void;
}): ReactElement {
  const [draft, setDraft] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [posted, setPosted] = useState(false);
  useEffect(() => {
    if (!posted) {
      return undefined;
    }
    const timer = setTimeout(() => setPosted(false), POSTED_MS);
    return () => clearTimeout(timer);
  }, [posted]);

  const send = async (): Promise<void> => {
    const body = draft.trim();
    if (!body || busy) {
      return;
    }
    setBusy(true);
    props.onSending(body);
    setDraft("");
    setError(null);
    setPosted(false);
    try {
      await props.onComment(body, props.replyTo?.id);
      setPosted(true);
      props.onCancelReply();
    } catch (failure) {
      // Not posted: the text comes back, with Retry.
      setDraft(body);
      const why = failure instanceof Error ? failure.message : String(failure);
      setError(`Your comment wasn't posted. ${why}`);
    } finally {
      props.onSending(null);
      setBusy(false);
    }
  };
  const onKeyDown = (event: KeyboardEvent<HTMLTextAreaElement>): void => {
    if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) {
      event.preventDefault();
      void send();
    }
  };
  return (
    <div className="drawer-composer">
      {props.replyTo === null ? null : (
        <p className="drawer-replying">
          Replying to {props.who(props.replyTo.author)}{" "}
          <button type="button" className="btn btn-small" onClick={props.onCancelReply}>
            Cancel
          </button>
        </p>
      )}
      <textarea
        rows={2}
        aria-label={`Comment on ${props.itemId}`}
        placeholder="Write a comment…"
        value={draft}
        onChange={(event) => setDraft(event.target.value)}
        onKeyDown={onKeyDown}
      />
      <button
        type="button"
        className="btn btn-small btn-primary"
        disabled={busy || draft.trim() === ""}
        onClick={() => void send()}
      >
        Send <span className="drawer-dim">⌘↩</span>
      </button>
      <p className="drawer-posted" role="status">
        {posted ? "✓ Posted. The assignee and the lead are told." : ""}
      </p>
      {error === null ? null : (
        <p className="drawer-error" role="alert">
          {error}{" "}
          <button type="button" className="btn btn-small" onClick={() => void send()}>
            Retry
          </button>
        </p>
      )}
    </div>
  );
}

export function Activity(props: {
  readonly detail: ItemDetail;
  readonly who: (actor: string) => string;
  readonly columnName: (key: string) => string;
  /** Absent where this connection can't comment. */
  readonly onComment?: PostComment;
}): ReactElement {
  const [filter, setFilter] = useState<Filter>("all");
  const [replyTo, setReplyTo] = useState<ItemComment | null>(null);
  const [sending, setSending] = useState<string | null>(null);
  const shown = entriesOf(props.detail, filter);
  const onReply = props.onComment === undefined ? undefined : setReplyTo;
  return (
    <div className="drawer-activity">
      <div className="drawer-filter" role="group" aria-label="Show">
        {(["all", "comments", "moves"] as const).map((f) => (
          <button
            key={f}
            type="button"
            className={filter === f ? "on" : ""}
            aria-pressed={filter === f}
            onClick={() => setFilter(f)}
          >
            {f === "all" ? "All" : f === "comments" ? "Comments" : "Moves"}
          </button>
        ))}
      </div>
      {shown.length === 0 && sending === null ? (
        <p className="drawer-empty">Nothing here yet.</p>
      ) : (
        <ol className="drawer-timeline" aria-label="Timeline">
          {shown.map((e) =>
            e.kind === "comment" ? (
              <CommentLine
                key={`c-${e.comment.id}`}
                comment={e.comment}
                all={props.detail.comments}
                who={props.who}
                onReply={onReply}
              />
            ) : (
              <li key={`e-${e.event.id}`} className="drawer-dim">
                {when(e.event.at)} {eventLine(e.event, props.who, props.columnName)}
              </li>
            ),
          )}
          {sending === null ? null : (
            <li className="drawer-owner drawer-sending">
              <span className="drawer-dim">Sending…</span> <b>{props.who("user")}</b>: {sending}
            </li>
          )}
        </ol>
      )}
      {props.onComment === undefined ? null : (
        <Composer
          itemId={props.detail.item?.id ?? ""}
          replyTo={replyTo}
          who={props.who}
          onCancelReply={() => setReplyTo(null)}
          onComment={props.onComment}
          onSending={setSending}
        />
      )}
    </div>
  );
}
