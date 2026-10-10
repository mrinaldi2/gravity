// Needs-you rows for pull requests (UX-051 "needs", decisions 5 and 11;
// H-269, H-284): "Review pull request #42 · H-247 …" once the bots have
// approved, "Re-check #45 …" after a change since you approved, which opens
// only that delta, and the merge rows: waiting for DevOps, and main moved
// outside the merge gate. Several PRs are one row each: no bulk approve.

import type { AttentionRowJson } from "../../protocol/dashboard";
import type { PullRequest } from "../../protocol/gen/hermes/pr/v1/pr_pb";
import { botsDone, recheckBase, reviewRowMeta } from "../prs/ownerText";
import { sha7 } from "../prs/prText";
import type { RowViewProps } from "./NeedsYou";

export interface PrRowActions {
  /** The project's PRs by number, to say who approved and spot a re-check. */
  readonly prs?: ReadonlyMap<number, PullRequest>;
  /** Opens the PR; `recheck` opens its delta since your approval. */
  readonly onOpenPr?: (pr: number, recheck: boolean) => void;
}

/** "Review PR #42 (H-247): Title" → "#42 · H-247 Title". */
function named(r: AttentionRowJson, pr: PullRequest | undefined): string {
  if (pr !== undefined) {
    return `#${pr.number} · ${pr.itemId} ${pr.itemTitle || pr.title}`;
  }
  const said = /^Review PR #(\d+) \(([^)]*)\): (.*)$/.exec(r.title);
  return said === null ? r.title : `#${said[1]} · ${said[2]} ${said[3]}`;
}

function open(
  a: PrRowActions,
  n: number | undefined,
  recheck: boolean,
): Pick<RowViewProps, "onAction"> & { readonly ok: boolean } {
  const go = a.onOpenPr;
  return {
    ok: go !== undefined && n !== undefined,
    onAction: () => {
      if (n !== undefined) {
        go?.(n, recheck);
      }
    },
  };
}

function reviewRow(r: AttentionRowJson, a: PrRowActions): RowViewProps | undefined {
  const n = r.pr_number;
  const pr = n === undefined ? undefined : a.prs?.get(n);
  // Your row comes after the bots (ruling 7629a873); the daemon holds it until
  // then, and a PR read since that still waits for a bot keeps it hidden.
  if (pr !== undefined && !botsDone(pr)) {
    return undefined;
  }
  const base = pr === undefined ? null : recheckBase(pr);
  if (pr !== undefined && base !== null) {
    const go = open(a, n, true);
    return {
      glyph: "⟳",
      tone: "you",
      title: `Re-check ${named(r, pr)}: conflict fixes since you approved`,
      meta: `${pr.author?.name ?? "The author"} changed it since you approved ${sha7(base)} · only that change`,
      action: go.ok ? "Re-check…" : undefined,
      label: `Re-check pull request #${pr.number}`,
      onAction: go.onAction,
      verb: "Re-check",
    };
  }
  const go = open(a, n, false);
  return {
    glyph: "⇄",
    tone: "you",
    title: `Review pull request ${named(r, pr)}`,
    meta: pr === undefined ? "Your review, after the reviewers" : reviewRowMeta(pr),
    action: go.ok ? "Review…" : undefined,
    label: `Review pull request #${n ?? ""}`,
    onAction: go.onAction,
    verb: "Review",
  };
}

/** The row for a pull request kind; undefined for any other kind. */
export function prRow(r: AttentionRowJson, a: PrRowActions): RowViewProps | undefined {
  switch (r.kind) {
    case "pr_review":
      return reviewRow(r, a);
    case "pr_merge_stuck": {
      const go = open(a, r.pr_number, false);
      return {
        glyph: "⏸",
        title: r.title,
        meta: "Waiting for DevOps to merge it",
        action: go.ok ? "Open" : undefined,
        label: `Open pull request #${r.pr_number ?? ""}`,
        onAction: go.onAction,
        verb: "Open",
      };
    }
    case "main_moved_outside": {
      const go = open(a, r.pr_number, false);
      return {
        glyph: "⚠",
        tone: "bad",
        title: r.title,
        meta: "Main moved outside the merge gate. Nothing was recorded; ask DevOps what happened.",
        action: go.ok ? "Open" : undefined,
        label: `Open pull request #${r.pr_number ?? ""}`,
        onAction: go.onAction,
        verb: "Open",
      };
    }
    default:
      return undefined;
  }
}
