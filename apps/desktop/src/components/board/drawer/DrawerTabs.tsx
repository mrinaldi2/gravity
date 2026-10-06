// The item drawer's three tabs (H-018 §3.6): Overview (description,
// acceptance criteria, verification, people, parent), Links grouped by kind,
// and Activity, comments and history as one timeline with the composer.

import { useState } from "react";
import type { KeyboardEvent, ReactElement } from "react";
import type {
  ItemComment,
  ItemEvent,
  ItemLink,
} from "../../../protocol/gen/hermes/board/v1/board_pb";
import {
  ItemEventKind,
  LinkKind,
  PersonRole,
  VerificationResult,
} from "../../../protocol/gen/hermes/board/v1/board_pb";
import type { ItemDetail } from "../../../protocol/gen/hermes/board/v1/requests_pb";
import Markdown from "../../control/Markdown";
import { eventLine, when } from "./drawerText";

const VERIFIED: Readonly<Record<VerificationResult, string>> = {
  [VerificationResult.UNSPECIFIED]: "◌",
  [VerificationResult.PASS]: "✓",
  [VerificationResult.FAIL]: "✗",
  [VerificationResult.BLOCKED]: "!",
};

export function Overview(props: {
  readonly detail: ItemDetail;
  readonly who: (actor: string) => string;
}): ReactElement {
  const item = props.detail.item;
  if (item === undefined) {
    return <p className="drawer-empty">This item has no details.</p>;
  }
  const ac = item.acceptanceCriteria;
  const reviewers = item.people.filter((p) => p.role === PersonRole.REVIEWER);
  const verifiers = item.people.filter((p) => p.role === PersonRole.VERIFIER);
  return (
    <div className="drawer-overview">
      {item.description.trim() ? (
        <Markdown>{item.description}</Markdown>
      ) : (
        <p className="drawer-empty">No description.</p>
      )}
      <h4>
        Acceptance criteria {ac.length ? `${ac.filter((c) => c.checked).length}/${ac.length}` : ""}
      </h4>
      {ac.length === 0 ? (
        <p className="drawer-empty">None yet.</p>
      ) : (
        <ul className="drawer-ac">
          {ac.map((c) => (
            <li key={c.idx}>
              <span aria-hidden="true">{c.checked ? "☑" : "☐"}</span>
              <span className="visually-hidden">{c.checked ? "Checked: " : "Not checked: "}</span>
              <span>{c.text}</span>
              {c.checked ? (
                <span className="drawer-dim">
                  {[c.checkedBy ? props.who(c.checkedBy) : "", c.machine, when(c.checkedAt)]
                    .filter(Boolean)
                    .join(" · ")}
                </span>
              ) : c.postInstall ? (
                <span className="drawer-dim">checked after install</span>
              ) : null}
            </li>
          ))}
        </ul>
      )}
      {item.verifications.length ? (
        <p className="drawer-line">
          <b>Verification</b>{" "}
          {item.verifications.map((v) => `${v.machine} ${VERIFIED[v.result]}`).join("  ")}
        </p>
      ) : null}
      <p className="drawer-line">
        <b>People</b> Assignee {item.assignee ? props.who(`bot:${item.assignee}`) : "none"}
        {reviewers.length
          ? ` · Reviewers ${reviewers.map((p) => props.who(`bot:${p.botId}`)).join(", ")}`
          : ""}
        {verifiers.length
          ? ` · Verifiers ${verifiers.map((p) => props.who(`bot:${p.botId}`)).join(", ")}`
          : ""}
      </p>
      {item.parentId ? (
        <p className="drawer-line">
          <b>Parent</b> <span className="mono">{item.parentId}</span>
        </p>
      ) : null}
    </div>
  );
}

const LINK_GROUPS: readonly { readonly title: string; readonly kinds: readonly LinkKind[] }[] = [
  { title: "Tasks", kinds: [LinkKind.TASK] },
  { title: "Decisions", kinds: [LinkKind.DECISION] },
  { title: "Artifacts", kinds: [LinkKind.ARTIFACT] },
  { title: "Branches", kinds: [LinkKind.BRANCH, LinkKind.PR] },
  { title: "Meetings", kinds: [LinkKind.MEETING] },
  {
    title: "Items",
    kinds: [LinkKind.ITEM_BLOCKS, LinkKind.ITEM_RELATES, LinkKind.ITEM_DUPLICATES],
  },
];

function linkText(link: ItemLink): string {
  const prefix =
    link.kind === LinkKind.ITEM_BLOCKS
      ? "blocks "
      : link.kind === LinkKind.ITEM_RELATES
        ? "relates "
        : link.kind === LinkKind.ITEM_DUPLICATES
          ? "duplicates "
          : link.kind === LinkKind.PR
            ? "PR "
            : "";
  const shown =
    link.kind === LinkKind.TASK || link.kind === LinkKind.DECISION
      ? link.ref.slice(0, 8)
      : link.ref;
  return `${prefix}${shown}${link.label ? ` · ${link.label}` : ""}`;
}

export function Links(props: { readonly links: readonly ItemLink[] }): ReactElement {
  if (props.links.length === 0) {
    return <p className="drawer-empty">Nothing linked yet.</p>;
  }
  return (
    <dl className="drawer-links">
      {LINK_GROUPS.map((group) => {
        const links = props.links.filter((l) => group.kinds.includes(l.kind));
        if (links.length === 0) {
          return null;
        }
        return (
          <div key={group.title} className="drawer-link-group">
            <dt>{group.title}</dt>
            {links.map((l) => (
              <dd key={`${l.kind}-${l.ref}`} title={l.ref}>
                {linkText(l)}
              </dd>
            ))}
          </div>
        );
      })}
    </dl>
  );
}

type Filter = "all" | "comments" | "moves";

type Entry =
  | { readonly kind: "comment"; readonly at: number; readonly comment: ItemComment }
  | { readonly kind: "event"; readonly at: number; readonly event: ItemEvent };

function stamp(at: { readonly seconds: bigint } | undefined): number {
  return at === undefined ? 0 : Number(at.seconds);
}

export function Activity(props: {
  readonly detail: ItemDetail;
  readonly who: (actor: string) => string;
  readonly columnName: (key: string) => string;
  /** Absent where this connection can't comment. */
  readonly onComment?: (body: string) => Promise<void>;
}): ReactElement {
  const [filter, setFilter] = useState<Filter>("all");
  const [draft, setDraft] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [sending, setSending] = useState(false);
  const entries: Entry[] = [
    ...props.detail.comments.map((c) => ({
      kind: "comment" as const,
      at: stamp(c.at),
      comment: c,
    })),
    ...props.detail.history
      .filter((e) => e.kind !== ItemEventKind.COMMENTED)
      .map((e) => ({ kind: "event" as const, at: stamp(e.at), event: e })),
  ];
  // Newest last, as a conversation reads.
  // oxlint-disable-next-line unicorn/no-array-sort
  entries.sort((a, b) => a.at - b.at);
  const shown = entries.filter(
    (e) =>
      filter === "all" ||
      (filter === "comments" && e.kind === "comment") ||
      (filter === "moves" && e.kind === "event" && e.event.kind === ItemEventKind.MOVED),
  );
  const send = async (): Promise<void> => {
    const body = draft.trim();
    if (!body || props.onComment === undefined) {
      return;
    }
    setSending(true);
    try {
      await props.onComment(body);
      setDraft("");
      setError(null);
    } catch (failure) {
      setError(failure instanceof Error ? failure.message : String(failure));
    } finally {
      setSending(false);
    }
  };
  const onKeyDown = (event: KeyboardEvent<HTMLTextAreaElement>): void => {
    if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) {
      event.preventDefault();
      void send();
    }
  };
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
      {shown.length === 0 ? (
        <p className="drawer-empty">Nothing here yet.</p>
      ) : (
        <ol className="drawer-timeline" aria-label="Timeline">
          {shown.map((e) =>
            e.kind === "comment" ? (
              <li
                key={`c-${e.comment.id}`}
                className={e.comment.author === "user" ? "drawer-owner" : ""}
              >
                <span className="drawer-dim">{when(e.comment.at)}</span>{" "}
                <b>{props.who(e.comment.author)}</b>: {e.comment.body}
              </li>
            ) : (
              <li key={`e-${e.event.id}`} className="drawer-dim">
                {when(e.event.at)} {eventLine(e.event, props.who, props.columnName)}
              </li>
            ),
          )}
        </ol>
      )}
      {props.onComment === undefined ? null : (
        <div className="drawer-composer">
          <textarea
            rows={2}
            aria-label={`Comment on ${props.detail.item?.id ?? ""}`}
            placeholder="Write a comment…"
            value={draft}
            onChange={(event) => setDraft(event.target.value)}
            onKeyDown={onKeyDown}
          />
          <button
            type="button"
            className="btn btn-small btn-primary"
            disabled={sending || draft.trim() === ""}
            onClick={() => void send()}
          >
            Send <span className="drawer-dim">⌘↩</span>
          </button>
          {error === null ? null : (
            <p className="drawer-error" role="alert">
              {error}
            </p>
          )}
        </div>
      )}
    </div>
  );
}
