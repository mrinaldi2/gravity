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
  reason: "install",
  release_id: "0.17.0",
  started_by: "bot:tester",
  started_at: "2026-10-06T09:00:00Z",
  deadline_at: "2026-10-06T09:30:00Z",
  phase: "paused",
};

function setup(open: Quiesce | null, grants?: readonly Grant[]) {
  const fake = new FakeDaemon();
  if (grants) {
    fake.grants = grants;
  }
  fake.onRequest("quiesce_status", () => ({ type: "quiesce", req_id: "1", quiesce: open }));
  fake.onRequest("quiesce_resume", () => ({ type: "quiesce", req_id: "1", quiesce: null }));
  render(<QuiesceLayer client={fake} connected addToast={vi.fn<AddToast>()} />);
  return fake;
}

describe("QuiesceBanner", () => {
  it("says every project is paused for the install and offers the owner Resume now", async () => {
    const user = userEvent.setup();
    const fake = setup(PAUSED, ["read", "control", "approve"]);
    expect(
      await screen.findByText(/Every project here is paused for the install of 0.17.0/),
    ).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Resume now" }));
    expect(fake.requests.some((r) => r.body.type === "quiesce_resume")).toBe(true);
    act(() => fake.emit("quiesce_update", { type: "quiesce_update", quiesce: null }));
    expect(screen.queryByText(/Every project here is paused/)).toBeNull();
  });

  it("shows nothing when nothing is paused, and appears when a pause starts", async () => {
    const fake = setup(null);
    await act(async () => {});
    expect(screen.queryByRole("status")).toBeNull();
    act(() => fake.emit("quiesce_update", { type: "quiesce_update", quiesce: PAUSED }));
    expect(screen.getByRole("status")).toHaveTextContent("paused for the install of 0.17.0");
  });

  it("leaves Resume now to the owner's approve grant", async () => {
    setup(PAUSED, ["read", "control"]);
    await screen.findByRole("status");
    expect(screen.queryByRole("button", { name: "Resume now" })).toBeNull();
  });
});
