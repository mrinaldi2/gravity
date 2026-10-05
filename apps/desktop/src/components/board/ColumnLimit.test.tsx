import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { AddToast } from "../../app/useToasts";
import { card, snapshot } from "../../test/boardFixtures";
import { FakeDaemon } from "../../test/fakeDaemon";
import { bot, project } from "../../test/fixtures";
import BoardView from "./BoardView";

const h24 = card({ id: "H-024", title: "Releases tab", columnKey: "ready" });

/** A board whose snapshots say whether this connection may rule it. */
function setup(canRule: boolean): FakeDaemon {
  const fake = new FakeDaemon()
    .onBoard("boardWatch", () => ({ case: "board", value: { ...snapshot([h24], 1n), canRule } }))
    .onBoard("boardGet", () => ({ case: "board", value: { ...snapshot([h24], 2n), canRule } }))
    .onBoard("columnSetLimit", () => ({ case: "board", value: snapshot([h24], 2n) }));
  render(
    <BoardView
      client={fake}
      project={project()}
      bots={[bot({ id: "b1", name: "Desktop Dev" })]}
      connected
      canControl
      addToast={vi.fn<AddToast>()}
    />,
  );
  return fake;
}

describe("ColumnLimit", () => {
  it("lets the owner set a column's WIP limit on the board's home", async () => {
    const user = userEvent.setup();
    const fake = setup(true);
    const ready = await screen.findByRole("region", { name: /^Ready/ });
    await user.click(within(ready).getByRole("button", { name: "WIP limit for Ready" }));
    const input = within(ready).getByRole("spinbutton", { name: "WIP limit for Ready" });
    await user.clear(input);
    await user.type(input, "12");
    await user.click(within(ready).getByRole("button", { name: "Save" }));
    expect(fake.boardCalls.find((c) => c.case === "columnSetLimit")?.value).toMatchObject({
      columnKey: "ready",
      wipLimit: 12,
    });
    await waitFor(() => {
      expect(screen.queryByRole("spinbutton")).toBeNull();
    });
    const after = screen.getByRole("region", { name: /^Ready/ });
    expect(within(after).getByRole("button", { name: "WIP limit for Ready" })).toBeInTheDocument();
  });

  it("clears the limit when left empty, and refuses anything but a whole number", async () => {
    const user = userEvent.setup();
    const fake = setup(true);
    const ready = await screen.findByRole("region", { name: /^Ready/ });
    await user.click(within(ready).getByRole("button", { name: "WIP limit for Ready" }));
    const input = within(ready).getByRole("spinbutton", { name: "WIP limit for Ready" });
    await user.clear(input);
    await user.type(input, "-2");
    await user.click(within(ready).getByRole("button", { name: "Save" }));
    expect(within(ready).getByRole("alert")).toHaveTextContent("A whole number");
    await user.clear(input);
    await user.click(within(ready).getByRole("button", { name: "Save" }));
    const call = fake.boardCalls.find((c) => c.case === "columnSetLimit");
    expect(call?.value).toMatchObject({ columnKey: "ready", wipLimit: undefined });
  });

  it("offers no limit editor where the board can't be ruled", async () => {
    setup(false);
    const ready = await screen.findByRole("region", { name: /^Ready/ });
    expect(within(ready).queryByRole("button", { name: "WIP limit for Ready" })).toBeNull();
  });
});
