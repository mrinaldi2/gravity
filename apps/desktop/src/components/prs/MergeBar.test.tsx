import { act, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { PullRequest } from "../../protocol/gen/hermes/pr/v1/pr_pb";
import { PrState } from "../../protocol/gen/hermes/pr/v1/pr_pb";
import { mergingPr } from "../../test/ownerReviewFixtures";
import { PR_NOW } from "../../test/prFixtures";
import MergeBar from "./MergeBar";
import type { OwnerAct } from "./usePrOwner";

const owner: OwnerAct = {
  busy: false,
  error: null,
  act: () => Promise.resolve(true),
  clear: () => undefined,
};

function live(): HTMLElement {
  const region = document.querySelector<HTMLElement>("[aria-live=polite]");
  if (region === null) {
    throw new Error("no live region");
  }
  return region;
}

function bar(pr: PullRequest, ticking = false) {
  return <MergeBar pr={pr} now={PR_NOW} live={ticking} canUndo owner={owner} />;
}

function settled(pr: PullRequest, state: PrState): PullRequest {
  return { ...pr, state, mergeAt: undefined };
}

afterEach(() => {
  vi.useRealTimers();
});

describe("The merge bar's announcements (UX-053 should-fix 2 and 3)", () => {
  it("announces the window once, not every second, and hides the counter", () => {
    vi.useFakeTimers({ now: PR_NOW });
    render(bar(mergingPr(10), true));
    const status = live();
    expect(status).toHaveTextContent(/^Merging #42 into main in 10 seconds\. Undo is available\.$/);
    const counter = screen.getByText(/^in 10 s$/);
    expect(counter).toHaveAttribute("aria-hidden", "true");

    act(() => vi.advanceTimersByTime(3000));
    expect(screen.getByText(/^in 7 s$/)).toHaveAttribute("aria-hidden", "true");
    expect(status).toHaveTextContent(/^Merging #42 into main in 10 seconds\. Undo is available\.$/);
  });

  it("names Undo with the PR it withdraws your approval of", () => {
    render(bar(mergingPr(7)));
    expect(screen.getByRole("button", { name: "Undo your approval of #42" })).toHaveTextContent(
      "Undo",
    );
  });

  it("announces Undone when the window closes back to open", () => {
    const merging = mergingPr(7);
    const { rerender } = render(bar(merging));
    rerender(bar(settled(merging, PrState.OPEN)));
    expect(live()).toHaveTextContent(/^Undone\.$/);
    expect(screen.queryByRole("button", { name: /Undo/ })).toBeNull();
  });

  it("announces Waiting for DevOps after the window, then Merged", () => {
    const merging = mergingPr(7);
    const { rerender } = render(bar(merging));
    rerender(bar(mergingPr(-1)));
    expect(live()).toHaveTextContent(/^Waiting for DevOps to merge #42\.$/);
    rerender(bar(settled(merging, PrState.MERGED)));
    expect(live()).toHaveTextContent(/^Merged\.$/);
  });

  it("says nothing for a PR that was never in a window", () => {
    render(bar(settled(mergingPr(7), PrState.MERGED)));
    expect(live()).toHaveTextContent(/^$/);
  });
});
