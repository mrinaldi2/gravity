import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { OwnerAction } from "../../protocol/ownerActions";
import { FakeDaemon } from "../../test/fakeDaemon";
import { ownerAction } from "../../test/ownerActionFixtures";
import { actionToastSpy } from "../../test/spies";
import OwnerActionConfirm, { HOLD_MS } from "./OwnerActionConfirm";
import OwnerActionList from "./OwnerActionList";
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
  it("shows the command verbatim, who proposed it and its hash", async () => {
    renderList(daemon([ownerAction()]));
    expect(await screen.findByText("brew services stop colima")).toBeInTheDocument();
    expect(screen.getByText("Waiting for you")).toBeInTheDocument();
    expect(screen.getByText(/Proposed by DevOps · sha 9f86d081884c/)).toBeInTheDocument();
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
    expect(await screen.findByText("Running")).toBeInTheDocument();
  });

  it("rejects with a reason", async () => {
    const client = daemon([ownerAction()]);
    renderList(client);
    await userEvent.click(await screen.findByRole("button", { name: "Reject…" }));
    await userEvent.type(screen.getByRole("textbox", { name: "Why not (optional)" }), "not now");
    await userEvent.click(screen.getByRole("button", { name: "Reject" }));
    expect(await screen.findByText("Rejected")).toBeInTheDocument();
    expect(client.requests.at(-1)?.body).toEqual({
      type: "owner_action_reject",
      id: "oa-1",
      reason: "not now",
    });
  });

  it("streams output while it runs and keeps up with pushes", async () => {
    const client = daemon([ownerAction()]);
    renderList(client);
    await screen.findByText("Waiting for you");
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
    expect(screen.getByText("Running")).toBeInTheDocument();
    expect(screen.getByText(/stopping\s+done/)).toBeInTheDocument();
  });

  it("shows only waiting commands on the dashboard", async () => {
    renderList(
      daemon([
        ownerAction(),
        ownerAction({ id: "oa-2", state: "succeeded", content: "echo old", exit_code: 0 }),
      ]),
      { projectId: "p1", waitingOnly: true },
    );
    expect(await screen.findByText("brew services stop colima")).toBeInTheDocument();
    expect(screen.queryByText("echo old")).not.toBeInTheDocument();
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
        onRun={onRun}
        onCancel={vi.fn<() => void>()}
      />,
    );
    const pre = screen.getByText(/echo 1/);
    fireEvent.scroll(pre, { target: { scrollTop: 100 } });
    expect(screen.getByText("Scroll to the end to run it.")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Run it" })).toBeDisabled();
    fireEvent.scroll(pre, { target: { scrollTop: 1700 } });
    expect(screen.queryByText("Scroll to the end to run it.")).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Run it" }));
    expect(onRun).toHaveBeenCalledOnce();
  });

  it("needs a press held for the full time on a touch screen", () => {
    vi.useFakeTimers();
    const onRun = vi.fn<() => void>();
    render(
      <OwnerActionConfirm
        action={ownerAction()}
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
