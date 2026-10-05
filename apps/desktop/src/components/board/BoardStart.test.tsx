import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { AddToast } from "../../app/useToasts";
import { DaemonError } from "../../protocol/connection";
import { snapshot } from "../../test/boardFixtures";
import { FakeDaemon } from "../../test/fakeDaemon";
import { project } from "../../test/fixtures";
import BoardView from "./BoardView";

const LINKED = "This project is linked with another computer.";

function show(fake: FakeDaemon): void {
  render(
    <BoardView
      client={fake}
      project={project()}
      bots={[]}
      connected
      canControl
      addToast={vi.fn<AddToast>()}
    />,
  );
}

describe("starting a board (H-037)", () => {
  it("lets the owner start a linked project's board on this computer", async () => {
    const user = userEvent.setup();
    let enabled = false;
    const fake = new FakeDaemon()
      .onBoard("boardWatch", () => {
        if (!enabled) {
          throw new DaemonError("no_board", LINKED);
        }
        return { case: "board", value: snapshot([], 1n) };
      })
      .onBoard("boardEnable", () => {
        enabled = true;
        return { case: "board", value: snapshot([], 1n) };
      });
    fake.grants = ["read", "control", "approve"];
    show(fake);
    await user.click(
      await screen.findByRole("button", { name: "Start the board on this computer" }),
    );
    expect(await screen.findByText("No items yet")).toBeInTheDocument();
    expect(fake.boardCalls.map((call) => call.case)).toContain("boardEnable");
  });

  it("offers it only to the owner", async () => {
    const fake = new FakeDaemon().onBoard("boardWatch", () => {
      throw new DaemonError("no_board", LINKED);
    });
    show(fake);
    expect(await screen.findByText(LINKED)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Start the board on this computer" })).toBeNull();
  });
});
