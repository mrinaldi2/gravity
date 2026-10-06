import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import type { ReleaseMachines } from "../../protocol/releases";
import { FakeDaemon } from "../../test/fakeDaemon";
import { actionToastSpy } from "../../test/spies";
import TestedOn from "./TestedOn";

const NAMES = new Map([
  ["t-here", "Tester"],
  ["t-imac", "Tester iMac"],
  ["t-win", "Tester Win"],
]);

function machines(set: readonly string[], by: "owner" | "lead" = "owner"): ReleaseMachines {
  const all = ["imac", "mac", "win-pc"];
  return {
    project_id: "p1",
    machine_name: "mac",
    deploys_to: set.length > 0 && by === "owner" ? set : all,
    set_by: set.length > 0 ? by : null,
    required: set.length > 0 ? set : all,
    set,
    testers: [
      { bot_id: "t-here", machine: "mac" },
      { bot_id: "t-imac", machine: "imac" },
      { bot_id: "t-win", machine: "win-pc" },
    ],
  };
}

function setup(canApprove: boolean) {
  let current = machines([]);
  const fake = new FakeDaemon()
    .onRequest("release_machines", () => ({
      type: "release_machines",
      req_id: "1",
      machines: current,
    }))
    .onRequest("release_machines_set", (body) => {
      current = machines((body as { machines: readonly string[] }).machines);
      return { type: "release_machines", req_id: "2", machines: current };
    });
  render(
    <TestedOn
      client={fake}
      projectId="p1"
      connected
      canApprove={canApprove}
      botName={(id) => NAMES.get(id)}
      addToast={actionToastSpy()}
    />,
  );
  return fake;
}

describe("TestedOn", () => {
  it("lists every tester's computer, each required by default", async () => {
    setup(true);
    const section = await screen.findByRole("region", { name: "Tested on" });
    expect(section).toHaveTextContent(
      "Each package needs a pass on every tester's computer, and goes to all of them.",
    );
    const boxes = within(section).getAllByRole("checkbox");
    expect(boxes.map((b) => (b as HTMLInputElement).checked)).toEqual([true, true, true]);
    expect(section).toHaveTextContent("mac · Tester");
    expect(section).toHaveTextContent("imac · Tester iMac");
    expect(section).toHaveTextContent("This computer is called mac.");
  });

  it("says a lead's list narrows testing, never where packages go", async () => {
    const fake = new FakeDaemon().onRequest("release_machines", () => ({
      type: "release_machines",
      req_id: "1",
      machines: machines(["imac"], "lead"),
    }));
    render(
      <TestedOn
        client={fake}
        projectId="p1"
        connected
        canApprove
        botName={(id) => NAMES.get(id)}
        addToast={actionToastSpy()}
      />,
    );
    const section = await screen.findByRole("region", { name: "Tested on" });
    expect(section).toHaveTextContent(
      "The lead chose where packages are tested; they still go to every tester's computer.",
    );
  });

  it("lets the owner leave a computer out, and go back to all", async () => {
    const fake = setup(true);
    const section = await screen.findByRole("region", { name: "Tested on" });
    await userEvent.click(within(section).getByRole("checkbox", { name: /win-pc/ }));
    expect(fake.requests.at(-1)?.body).toEqual({
      type: "release_machines_set",
      project_id: "p1",
      machines: ["imac", "mac"],
    });
    expect(section).toHaveTextContent("the computers you chose");
    await userEvent.click(
      within(section).getByRole("button", { name: "Use every tester's computer" }),
    );
    expect(fake.requests.at(-1)?.body).toEqual({
      type: "release_machines_set",
      project_id: "p1",
      machines: [],
    });
  });

  it("shows the list read-only without the approve grant", async () => {
    setup(false);
    const section = await screen.findByRole("region", { name: "Tested on" });
    for (const box of within(section).getAllByRole("checkbox")) {
      expect(box).toBeDisabled();
    }
  });
});
