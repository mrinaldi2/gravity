import { create } from "@bufbuild/protobuf";
import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { PrPushSchema } from "../../protocol/gen/hermes/pr/v1/pr_pb";
import { CARD_PROJECT } from "../../test/cardFixtures";
import type { FakeDaemon } from "../../test/fakeDaemon";
import { mergingPr, ownerDaemon, recheckPr } from "../../test/ownerReviewFixtures";
import { HEAD, OLDER, PR_NOW, readyForYouPr, waitingPr } from "../../test/prFixtures";
import PullRequestsView from "./PullRequestsView";

function show(
  client: FakeDaemon,
  initialNumber = 42,
  extra: { recheck?: boolean; onOpenSettings?: () => void } = {},
) {
  return render(
    <PullRequestsView
      client={client}
      project={CARD_PROJECT}
      connected
      now={PR_NOW}
      initialNumber={initialNumber}
      live={false}
      {...extra}
    />,
  );
}

function sent(client: FakeDaemon, type: string) {
  return client.requests.filter((r) => r.body.type === type).map((r) => r.body);
}

const reads = (client: FakeDaemon): number =>
  client.prCalls.filter((c) => c.case === "prGet").length;

describe("Your review: Approve and Ask for changes (AC1)", () => {
  it("approves the head over the app's connection, saying what it triggers", async () => {
    const client = ownerDaemon();
    show(client);
    const review = await screen.findByRole("button", { name: "Review…" });
    await userEvent.click(review);
    const dialog = screen.getByRole("dialog", { name: "Your review of #42" });
    // It opens on Approve, which names what it does: only you are missing.
    expect(within(dialog).getByRole("radio", { name: /^Approve/ })).toHaveFocus();
    expect(dialog).toHaveTextContent(
      "#42 merges into main and ships in the next release. You have 10 seconds to undo.",
    );
    const before = reads(client);
    await userEvent.click(within(dialog).getByRole("button", { name: "Approve #42" }));
    expect(sent(client, "pr_review_submit")).toEqual([
      { type: "pr_review_submit", project_id: "p1", number: 42, sha: HEAD, verdict: "approved" },
    ]);
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    // The PR is read again, and focus returns to Review….
    expect(reads(client)).toBeGreaterThan(before);
    expect(screen.getByRole("button", { name: "Review…" })).toHaveFocus();
  });

  it("names who else it waits for after you", async () => {
    show(ownerDaemon([waitingPr()]));
    await userEvent.click(await screen.findByRole("button", { name: "Review…" }));
    expect(screen.getByRole("dialog")).toHaveTextContent(
      "When CE approves too, #42 merges into main and ships in the next release.",
    );
  });

  it("asks for changes only with a note, which goes to the author", async () => {
    const client = ownerDaemon();
    show(client);
    await userEvent.click(await screen.findByRole("button", { name: "Review…" }));
    const dialog = screen.getByRole("dialog");
    await userEvent.click(within(dialog).getByRole("radio", { name: /^Ask for changes/ }));
    expect(dialog).toHaveTextContent("Desktop Dev gets your note; the card goes back to Doing.");
    const send = within(dialog).getByRole("button", { name: "Ask for changes on #42" });
    expect(send).toBeDisabled();
    await userEvent.type(
      within(dialog).getByLabelText("Note (needed for changes)"),
      "Keep Reject out of focus",
    );
    await userEvent.click(send);
    expect(sent(client, "pr_review_submit")).toEqual([
      {
        type: "pr_review_submit",
        project_id: "p1",
        number: 42,
        sha: HEAD,
        verdict: "changes_requested",
        summary: "Keep Reject out of focus",
      },
    ]);
  });

  it("turns Approve off once a newer commit arrives, and Escape returns focus", async () => {
    const pr = readyForYouPr();
    const client = ownerDaemon([pr]);
    show(client);
    const review = await screen.findByRole("button", { name: "Review…" });
    await userEvent.click(review);
    pr.headSha = "aaaabbbbccccdddd";
    act(() => {
      client.emitPrPush(
        create(PrPushSchema, {
          push: { case: "prUpdated", value: { projectId: "p1", number: 42 } },
        }),
      );
    });
    const dialog = screen.getByRole("dialog");
    await waitFor(() =>
      expect(within(dialog).getByRole("radio", { name: /^Approve/ })).toBeDisabled(),
    );
    expect(dialog).toHaveTextContent(
      "New commits came in: aaaabbb is the latest. Review it again.",
    );
    await userEvent.keyboard("{Escape}");
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(screen.getByRole("button", { name: "Review…" })).toHaveFocus();
  });

  it("offers no review without the approve grant", async () => {
    const client = ownerDaemon();
    client.grants = ["read", "control"];
    show(client);
    await screen.findByRole("heading", { name: /#42/ });
    expect(screen.queryByRole("button", { name: "Review…" })).toBeNull();
  });
});

describe("The 10 s Undo (AC2)", () => {
  it("withdraws your approval in the window", async () => {
    const client = ownerDaemon([mergingPr(7)]);
    show(client);
    expect(await screen.findByText(/Merging #42 into main in 7 s/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Undo" }));
    expect(sent(client, "pr_merge_undo")).toEqual([
      { type: "pr_merge_undo", project_id: "p1", number: 42 },
    ]);
  });

  it("says it waits for DevOps once the window is over", async () => {
    show(ownerDaemon([mergingPr(-3)]));
    expect(await screen.findByText(/Waiting for DevOps to merge #42 into main/)).toBeDefined();
    expect(screen.queryByRole("button", { name: "Undo" })).toBeNull();
  });
});

describe("Line comments", () => {
  it("shows threads under their lines, and replies, resolves and starts one", async () => {
    const client = ownerDaemon();
    show(client);
    await userEvent.click(await screen.findByRole("tab", { name: /^Files/ }));
    const thread = await screen.findByRole("group", { name: /Comments on .*ReleaseReview.tsx:50/ });
    expect(thread).toHaveTextContent("UX Designer · Should-fix");
    expect(thread).toHaveTextContent("Kept the heading's tabIndex -1.");
    expect(screen.getByText(/✓ Resolved · UX Designer on line 49/)).toBeInTheDocument();
    expect(screen.getByText("This helper moved.")).toBeInTheDocument();

    await userEvent.click(within(thread).getByRole("button", { name: "Reply" }));
    await userEvent.type(within(thread).getByRole("textbox"), "Thanks");
    await userEvent.click(within(thread).getByRole("button", { name: "Comment" }));
    await userEvent.click(within(thread).getByRole("button", { name: "Resolve" }));

    const diff = screen.getByRole("region", { name: /Diff of .*ReleaseReview.tsx/ });
    await userEvent.click(within(diff).getByRole("button", { name: "Comment on line 52" }));
    await userEvent.type(within(diff).getByRole("textbox", { name: "Comment on line 52" }), "Why?");
    await userEvent.selectOptions(within(diff).getByLabelText("Kind of comment"), "must");
    const comments = within(diff).getAllByRole("button", { name: "Comment" });
    await userEvent.click(comments[comments.length - 1] as HTMLElement);

    expect(sent(client, "pr_comment_add")).toEqual([
      {
        type: "pr_comment_add",
        project_id: "p1",
        number: 42,
        sha: HEAD,
        body: "Thanks",
        reply_to: "c1",
      },
      {
        type: "pr_comment_add",
        project_id: "p1",
        number: 42,
        sha: HEAD,
        path: "apps/desktop/src/components/releases/ReleaseReview.tsx",
        line: 52,
        side: "new",
        body: "Why?",
        severity: "must",
      },
    ]);
    expect(sent(client, "pr_comment_resolve")).toEqual([
      { type: "pr_comment_resolve", project_id: "p1", number: 42, comment_id: "c1" },
    ]);
  });
});

describe("Re-check and the Owner review line (AC3, AC4)", () => {
  it("opens a re-check on the delta since your approval", async () => {
    const client = ownerDaemon([recheckPr()]);
    show(client, 45, { recheck: true });
    const banner = (await screen.findByText(/Only what changed since you approved/)).closest(
      '[role="status"]',
    );
    expect(banner).toHaveTextContent("since you approved c79c8b5 · 3 files");
    expect(client.prCalls).toContainEqual(
      expect.objectContaining({
        case: "prDiff",
        value: expect.objectContaining({ fromSha: OLDER }),
      }),
    );
    expect(screen.getByRole("button", { name: "Review…" })).toBeInTheDocument();
  });

  it("shows the setting on the Waiting for line, with Change", async () => {
    const onOpenSettings = vi.fn<() => void>();
    show(ownerDaemon([waitingPr()]), 42, { onOpenSettings });
    const line = await screen.findByText(/Owner review: every pull request/);
    await userEvent.click(within(line).getByRole("button", { name: "Change" }));
    expect(onOpenSettings).toHaveBeenCalled();
  });
});
