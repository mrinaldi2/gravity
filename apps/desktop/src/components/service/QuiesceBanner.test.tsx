import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { AddToast } from "../../app/useToasts";
import type { Grant } from "../../protocol/entities";
import type { Quiesce } from "../../protocol/quiesce";
import { FakeDaemon } from "../../test/fakeDaemon";
import QuiesceLayer from "./QuiesceBanner";

const PAUSED: Quiesce = {
  id: "q1",
  reason: "install of 0.17.0",
  release_id: "0.17.0",
  version: "0.17.0",
  started_by: "bot:tester",
  started_at: "2026-10-06T09:00:00Z",
  deadline_at: "2026-10-06T09:30:00Z",
  phase: "paused",
};

function setup(open: Quiesce | null, grants?: readonly Grant[], ended: Quiesce | null = null) {
  const fake = new FakeDaemon();
  if (grants) {
    fake.grants = grants;
  }
  fake.onRequest("quiesce_status", () => ({ type: "quiesce", req_id: "1", quiesce: open, ended }));
  fake.onRequest("quiesce_resume", () => ({ type: "quiesce", req_id: "1", quiesce: null }));
  const addToast = vi.fn<AddToast>();
  render(<QuiesceLayer client={fake} connected addToast={addToast} />);
  return { fake, addToast };
}

describe("QuiesceBanner", () => {
  it("says every project on this computer is paused and resumes only after a confirm", async () => {
    const user = userEvent.setup();
    const { fake } = setup(PAUSED, ["read", "control", "approve"]);
    expect(
      await screen.findByText(
        /Every project on this computer is paused while The Hermes 0.17.0 installs/,
      ),
    ).toBeInTheDocument();
    expect(screen.getByRole("status")).toHaveTextContent(
      "They pick up where they left off when the install is done, or at",
    );
    await user.click(screen.getByRole("button", { name: "Resume now" }));
    expect(screen.getByRole("dialog")).toHaveTextContent(
      "The install of 0.17.0 is still running. If bots start working now, it may fail",
    );
    expect(screen.getByRole("button", { name: "Cancel" })).toHaveFocus();
    expect(fake.requests.some((r) => r.body.type === "quiesce_resume")).toBe(false);
    const confirm = screen.getAllByRole("button", { name: "Resume now" }).at(-1);
    await user.click(confirm as HTMLElement);
    expect(fake.requests.some((r) => r.body.type === "quiesce_resume")).toBe(true);
    act(() => fake.emit("quiesce_update", { type: "quiesce_update", quiesce: null }));
    expect(screen.queryByText(/Every project on this computer is paused/)).toBeNull();
  });

  it("shows nothing when nothing is paused, and appears when a pause starts", async () => {
    const { fake } = setup(null);
    await act(async () => {});
    expect(screen.queryByRole("status")).toBeNull();
    act(() => fake.emit("quiesce_update", { type: "quiesce_update", quiesce: PAUSED }));
    expect(screen.getByRole("status")).toHaveTextContent("while The Hermes 0.17.0 installs");
  });

  it("leaves Resume now to the owner's approve grant", async () => {
    setup(PAUSED, ["read", "control"]);
    await screen.findByRole("status");
    expect(screen.queryByRole("button", { name: "Resume now" })).toBeNull();
  });

  it("names what holds the folder while the install waits", async () => {
    setup({
      ...PAUSED,
      phase: "blocked",
      report: {
        unresolved: [
          { pid: 4120, command: "node", bot_name: "Unity", project_name: "PhD" },
          { pid: 77, command: "sleep" },
        ],
      },
    });
    const banner = await screen.findByRole("status");
    expect(banner).toHaveTextContent(
      "The install is waiting for these programs to close the Hermes folder:",
    );
    expect(screen.getByText("node (process 4120) · Unity in PhD")).toBeInTheDocument();
    expect(screen.getByText("sleep (process 77)")).toBeInTheDocument();
    expect(banner).toHaveTextContent("Quit them to let the install go ahead.");
  });

  it("says when the list of background services changed", async () => {
    setup({ ...PAUSED, report: { services_changed: true } });
    expect(await screen.findByRole("status")).toHaveTextContent(
      "The list of background services changed after the Hermes service started.",
    );
  });

  it("tells how the last install went, once", async () => {
    const ended: Quiesce = {
      ...PAUSED,
      id: "q-done",
      report: { resumed: { outcome: "rolled_back", running_version: "0.16.3" } },
    };
    const { addToast } = setup(null, undefined, ended);
    await act(async () => {});
    expect(addToast).toHaveBeenCalledWith(
      "warn",
      "The install of 0.17.0 didn't work, so this computer went back to 0.16.3. Everything is running again.",
      "",
    );
  });
});
