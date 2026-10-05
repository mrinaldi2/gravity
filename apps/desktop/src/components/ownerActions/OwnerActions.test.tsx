import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { OwnerAction } from "../../protocol/ownerActions";
import { FakeDaemon } from "../../test/fakeDaemon";
import { ownerAction } from "../../test/ownerActionFixtures";
import { actionToastSpy } from "../../test/spies";
import OwnerActionConfirm, { HOLD_MS } from "./OwnerActionConfirm";
import OwnerActionList from "./OwnerActionList";
import { resultLine } from "./ownerActionText";
import type { OwnerActionScope } from "./useOwnerActions";

function daemon(actions: readonly OwnerAction[], owner = true): FakeDaemon {
  const client = new FakeDaemon()
    .onRequest("owner_action_list", () => ({ type: "owner_actions", req_id: "1", actions }))
    .onRequest("owner_action_run", () => ({
      type: "owner_action",
      req_id: "1",
      action: { ...(actions[0] ?? ownerAction()), state: "running" },
    }))
    .onRequest("owner_action_reject", () => ({
      type: "owner_action",
      req_id: "1",
      action: { ...(actions[0] ?? ownerAction()), state: "rejected", reject_reason: "not now" },
    }));
  client.grants = owner ? ["read", "control", "approve"] : ["read", "control"];
  return client;
}

function renderList(
  client: FakeDaemon,
  scope: OwnerActionScope = { projectId: "p1", itemId: "H-117" },
) {
  return render(
    <OwnerActionList
      client={client}
      connected
      scope={scope}
      addToast={actionToastSpy()}
      botName={(id) => (id === "ops" ? "DevOps" : "a bot")}
    />,
  );
}

afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
});

describe("OwnerActionList", () => {
  it("says who asks for what and where, why, and its fingerprint", async () => {
    renderList(daemon([ownerAction()]));
    expect(await screen.findByText("brew services stop colima")).toBeInTheDocument();
    expect(
      screen.getByRole("article", {
        name: "DevOps asks you to run a command on this computer: Colima holds the old Hermes service program open",
      }),
    ).toBeInTheDocument();
    expect(
      screen.getByText("Why: Colima holds the old Hermes service program open"),
    ).toBeInTheDocument();
    expect(screen.getByText("Fingerprint 9f86d081")).toBeInTheDocument();
  });

  it("renders nothing outside its scope", async () => {
    const client = daemon([ownerAction({ item_id: "H-001" })]);
    const { container } = renderList(client);
    await waitFor(() => expect(client.requests).toHaveLength(1));
    expect(container).toBeEmptyDOMElement();
  });

  it("is read-only without the approve grant", async () => {
    renderList(daemon([ownerAction()], false));
    await screen.findByText("brew services stop colima");
    expect(screen.queryByRole("button", { name: "Run…" })).not.toBeInTheDocument();
    expect(
      screen.getByText("You can run this from a device with approve access."),
    ).toBeInTheDocument();
  });

  it("runs only after the confirm sheet, sending the hash it showed", async () => {
    const client = daemon([ownerAction()]);
    renderList(client);
    await userEvent.click(await screen.findByRole("button", { name: "Run…" }));
    const sheet = screen.getByRole("dialog", { name: "Run on this computer" });
    expect(sheet).toHaveTextContent("brew services stop colima");
    expect(client.requests.map((r) => r.body.type)).toEqual(["owner_action_list"]);
    await userEvent.click(screen.getByRole("button", { name: "Run it" }));
    await waitFor(() =>
      expect(client.requests.at(-1)?.body).toEqual({
        type: "owner_action_run",
        id: "oa-1",
        sha256: ownerAction().sha256,
      }),
    );
    expect(await screen.findByText("◑ Running on this computer…")).toBeInTheDocument();
  });

  it("rejects with a reason", async () => {
    const client = daemon([ownerAction()]);
    renderList(client);
    await userEvent.click(await screen.findByRole("button", { name: "Reject…" }));
    await userEvent.type(screen.getByRole("textbox", { name: "Why not (optional)" }), "not now");
    await userEvent.click(screen.getByRole("button", { name: "Reject" }));
    expect(await screen.findByText("⊘ You rejected it: not now")).toBeInTheDocument();
    expect(client.requests.at(-1)?.body).toEqual({
      type: "owner_action_reject",
      id: "oa-1",
      reason: "not now",
    });
  });

  it("streams output while it runs and keeps up with pushes", async () => {
    const client = daemon([ownerAction()]);
    renderList(client);
    await screen.findByText("brew services stop colima");
    act(() => {
      client.emit("owner_action_update", {
        type: "owner_action_update",
        action: ownerAction({ state: "running" }),
      });
      client.emit("owner_action_output", {
        type: "owner_action_output",
        id: "oa-1",
        chunk: "stopping\n",
      });
      client.emit("owner_action_output", {
        type: "owner_action_output",
        id: "oa-1",
        chunk: "done\n",
      });
    });
    expect(screen.getByText("◑ Running on this computer…")).toBeInTheDocument();
    expect(screen.getByText(/stopping\s+done/)).toBeInTheDocument();
  });

  it("shows only waiting and running commands on the dashboard, running first", async () => {
    renderList(
      daemon([
        ownerAction({ id: "oa-new", created_at: "2026-10-05T13:00:00Z", content: "echo newer" }),
        ownerAction(),
        ownerAction({ id: "oa-run", state: "running", content: "echo running" }),
        ownerAction({ id: "oa-2", state: "succeeded", content: "echo old", exit_code: 0 }),
      ]),
      { projectId: "p1", waitingOnly: true },
    );
    await screen.findByText("brew services stop colima");
    const order = screen.getAllByRole("article").map((a) => a.querySelector("pre")?.textContent);
    expect(order).toEqual(["echo running", "brew services stop colima", "echo newer"]);
    expect(screen.queryByText("echo old")).not.toBeInTheDocument();
  });

  it("opens the output of a run that failed", async () => {
    renderList(
      daemon([ownerAction({ state: "failed", exit_code: 1, output_tail: "no such service" })]),
    );
    const summary = await screen.findByText("Output");
    expect(summary.closest("details")).toHaveAttribute("open");
  });

  it("asks nothing of a daemon without owner actions", async () => {
    const client = new FakeDaemon();
    const { container } = renderList(client);
    await waitFor(() => expect(client.requests).toHaveLength(1));
    expect(container).toBeEmptyDOMElement();
  });
});

describe("OwnerActionConfirm", () => {
  it("keeps Run off until a long script is scrolled to its end", () => {
    // jsdom has no layout: give the script a height taller than its box.
    vi.spyOn(HTMLElement.prototype, "scrollHeight", "get").mockReturnValue(2000);
    vi.spyOn(HTMLElement.prototype, "clientHeight", "get").mockReturnValue(300);
    const onRun = vi.fn<() => void>();
    render(
      <OwnerActionConfirm
        action={ownerAction({ content: "echo 1\n".repeat(200) })}
        proposer="DevOps"
        onRun={onRun}
        onCancel={vi.fn<() => void>()}
      />,
    );
    const pre = screen.getByText(/echo 1/);
    fireEvent.scroll(pre, { target: { scrollTop: 100 } });
    const hint = "Scroll to the end of the script to run it.";
    expect(screen.getByText(hint)).toBeInTheDocument();
    const run = screen.getByRole("button", { name: "Run it" });
    expect(run).toBeDisabled();
    expect(run).toHaveAccessibleDescription(hint);
    fireEvent.scroll(pre, { target: { scrollTop: 1700 } });
    expect(screen.queryByText(hint)).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Run it" }));
    expect(onRun).toHaveBeenCalledOnce();
  });

  it("needs a press held for the full time on a touch screen", () => {
    vi.useFakeTimers();
    const onRun = vi.fn<() => void>();
    render(
      <OwnerActionConfirm
        action={ownerAction()}
        proposer="DevOps"
        onRun={onRun}
        onCancel={vi.fn<() => void>()}
        hold
      />,
    );
    const button = screen.getByRole("button", { name: "Hold to run" });
    fireEvent.pointerDown(button);
    act(() => vi.advanceTimersByTime(HOLD_MS / 2));
    fireEvent.pointerUp(button);
    act(() => vi.advanceTimersByTime(HOLD_MS));
    expect(onRun).not.toHaveBeenCalled();
    fireEvent.pointerDown(button);
    act(() => vi.advanceTimersByTime(HOLD_MS));
    expect(onRun).toHaveBeenCalledOnce();
  });
});

describe("OwnerActionConfirm, before it runs", () => {
  it("says why, as whom and that it's final, with Cancel focused and Esc closing it", () => {
    const onCancel = vi.fn<() => void>();
    render(
      <OwnerActionConfirm
        action={ownerAction({ target_name: "win-pc" })}
        proposer="DevOps"
        onRun={vi.fn<() => void>()}
        onCancel={onCancel}
      />,
    );
    expect(
      screen.getByText(
        "DevOps asks for this because: Colima holds the old Hermes service program open. It runs as you, with your permissions on win-pc, and can't be undone.",
      ),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Cancel" })).toHaveFocus();
    fireEvent.keyDown(screen.getByRole("dialog", { name: "Run on win-pc" }), { key: "Escape" });
    expect(onCancel).toHaveBeenCalledOnce();
  });
});

describe("result lines", () => {
  const ran = { run_at: "2026-10-05T12:00:00Z", finished_at: "2026-10-05T12:00:02Z" };
  it.each([
    [
      ownerAction({ ...ran, state: "succeeded", exit_code: 0 }),
      "✓ Done · exit 0 · took 2 s. DevOps has the output.",
    ],
    [
      ownerAction({ ...ran, state: "failed", exit_code: 1 }),
      "✗ Failed · exit 1 · took 2 s. DevOps has been told.",
    ],
    [
      ownerAction({ state: "timed_out" }),
      "⏱ Stopped after 10 minutes, its time limit. DevOps has been told.",
    ],
    [ownerAction({ state: "rejected" }), "⊘ You rejected it."],
    [ownerAction({ state: "withdrawn" }), "Withdrawn by DevOps."],
    [ownerAction({ state: "expired" }), "Expired: nobody answered within 24 hours."],
    [ownerAction({ state: "running", target_name: "win-pc" }), "◑ Running on win-pc…"],
  ])("%#", (action, line) => {
    expect(resultLine(action, "DevOps")).toBe(line);
  });
});
