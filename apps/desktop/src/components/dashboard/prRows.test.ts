import { describe, expect, it, vi } from "vitest";
import type { AttentionRowJson } from "../../protocol/dashboard";
import type { PullRequest } from "../../protocol/gen/hermes/pr/v1/pr_pb";
import { botsPendingPr, recheckPr } from "../../test/ownerReviewFixtures";
import { readyForYouPr } from "../../test/prFixtures";
import { attentionRow } from "./attentionRows";

const click = { currentTarget: document.createElement("button") } as never;

function actions(prs: readonly PullRequest[] = []) {
  return {
    onItem: vi.fn<(itemId: string, opener: HTMLElement) => void>(),
    onOpenPr: vi.fn<(pr: number, recheck: boolean) => void>(),
    prs: new Map(prs.map((pr) => [pr.number, pr])),
  };
}

function row(kind: AttentionRowJson["kind"], n: number, title: string): AttentionRowJson {
  return { kind, id: `${kind}:mac:pr-${n}`, title, pr_number: n };
}

describe("Needs you: pull requests (AC3)", () => {
  it("asks for your review once the bots approved, saying who and that it merges", () => {
    const a = actions([readyForYouPr()]);
    const view = attentionRow(row("pr_review", 42, "Review PR #42 (H-293): x"), a);
    expect(view?.title).toBe("Review pull request #42 · H-293 Waiting for you on releases");
    expect(view?.meta).toBe(
      "Desktop Dev · Architect ✓ UX ✓ CE ✓ · ✓ 5 checks · merges into main when you approve",
    );
    expect(view?.action).toBe("Review…");
    view?.onAction?.(click);
    expect(a.onOpenPr).toHaveBeenCalledWith(42, false);
  });

  it("re-checks a change since you approved, opening only that delta", () => {
    const a = actions([recheckPr()]);
    const view = attentionRow(row("pr_review", 45, "Review PR #45 (H-259): Search in Docs"), a);
    expect(view?.title).toBe(
      "Re-check #45 · H-259 Search in Docs: conflict fixes since you approved",
    );
    expect(view?.meta).toBe("Desktop Dev changed it since you approved c79c8b5 · only that change");
    expect(view?.action).toBe("Re-check…");
    view?.onAction?.(click);
    expect(a.onOpenPr).toHaveBeenCalledWith(45, true);
  });

  it("keeps the row back while a bot still has to approve", () => {
    const view = attentionRow(
      row("pr_review", 46, "Review PR #46 (H-293): x"),
      actions([botsPendingPr()]),
    );
    expect(view).toBeUndefined();
  });

  it("names the PR from the daemon's title before the PR is read", () => {
    const view = attentionRow(row("pr_review", 7, "Review PR #7 (H-9): Fix it"), actions());
    expect(view?.title).toBe("Review pull request #7 · H-9 Fix it");
    expect(view?.meta).toBe("Your review, after the reviewers");
  });

  it("shows a merge waiting for DevOps and main moved outside the gate", () => {
    const a = actions();
    const stuck = attentionRow(
      row("pr_merge_stuck", 42, "PR #42 (H-293) is waiting for DevOps to merge it; asked 2 times"),
      a,
    );
    expect(stuck?.meta).toBe("Waiting for DevOps to merge it");
    const moved = attentionRow(
      row("main_moved_outside", 42, "Main moved to PR #42's head f0678fc outside the merge gate"),
      a,
    );
    expect(moved?.tone).toBe("bad");
    moved?.onAction?.(click);
    expect(a.onOpenPr).toHaveBeenCalledWith(42, false);
  });

  it("offers no button without pull requests", () => {
    const view = attentionRow(row("pr_review", 42, "Review PR #42 (H-293): x"), {
      onItem: vi.fn<(itemId: string, opener: HTMLElement) => void>(),
    });
    expect(view?.action).toBeUndefined();
  });
});
