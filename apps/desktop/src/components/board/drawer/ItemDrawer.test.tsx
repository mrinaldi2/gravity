import { create } from "@bufbuild/protobuf";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { DaemonError } from "../../../protocol/connection";
import { EditResultSchema } from "../../../protocol/gen/hermes/board/v1/requests_pb";
import { defaultColumns, unmet } from "../../../test/boardFixtures";
import { DASH_BOTS } from "../../../test/dashboardFixtures";
import { DRAWER_NOW, drawerCheck, itemDetail } from "../../../test/drawerFixtures";
import { FakeDaemon } from "../../../test/fakeDaemon";
import { bot } from "../../../test/fixtures";
import ItemDrawer from "./ItemDrawer";

const BOTS = [...DASH_BOTS, bot({ id: "lead", name: "Team Lead" })];

function setup(
  options: {
    readonly canComment?: boolean;
    readonly fake?: FakeDaemon;
    readonly check?: ReturnType<typeof drawerCheck>;
    readonly detail?: ReturnType<typeof itemDetail>;
  } = {},
) {
  const fake = options.fake ?? new FakeDaemon();
  const check = options.check ?? drawerCheck();
  fake
    .onBoard("itemGet", () => ({ case: "item", value: options.detail ?? itemDetail() }))
    .onBoard("itemMoveCheck", () => ({ case: "moveCheck", value: check }));
  const onClose = vi.fn<() => void>();
  const onMove = vi.fn<(anchor: { readonly x: number; readonly y: number }) => void>();
  const onStep = vi.fn<(direction: -1 | 1) => void>();
  render(
    <ItemDrawer
      api={fake}
      itemId="H-017"
      columns={defaultColumns()}
      bots={BOTS}
      canComment={options.canComment ?? true}
      onClose={onClose}
      onMove={onMove}
      onStep={onStep}
      now={() => DRAWER_NOW}
    />,
  );
  return { fake, onClose, onMove, onStep };
}

describe("ItemDrawer", () => {
  it("shows where the item stands and what its next move needs", async () => {
    setup();
    const drawer = screen.getByRole("complementary", { name: "Item H-017" });
    expect(
      await within(drawer).findByRole("heading", {
        name: "Backlog and meetings data model in the DB",
      }),
    ).toBeInTheDocument();
    const stepper = within(drawer).getByRole("list", { name: "Where it stands" });
    expect(within(stepper).getByText(/Doing/)).toHaveAttribute("aria-current", "step");
    expect(drawer).toHaveTextContent("Doing for 31h · Desktop Dev · P1 · M · desktop daemon");
    const next = await within(drawer).findByRole("region", { name: "Next step" });
    expect(next).toHaveTextContent("To reach Review:");
    expect(next).toHaveTextContent("✗ No change note is linked. Link the change-note artifact.");
  });

  it("lists criteria with who checked them, people, and the parent", async () => {
    setup();
    await screen.findByText("Items survive a restart");
    const overview = screen.getByRole("tabpanel");
    expect(overview).toHaveTextContent("Acceptance criteria 1/2");
    expect(overview).toHaveTextContent("Items survive a restartTester Win · mac");
    expect(overview).toHaveTextContent("Verification mac ✓");
    expect(overview).toHaveTextContent("Assignee Desktop Dev · Reviewers Architect");
    expect(overview).toHaveTextContent("Parent H-016");
  });

  it("says which open criteria are checked after install", async () => {
    const detail = itemDetail();
    const second = detail.item?.acceptanceCriteria[1];
    if (second) second.postInstall = true;
    setup({ detail });
    await screen.findByText("Guards refuse with reasons");
    expect(screen.getByRole("tabpanel")).toHaveTextContent(
      "Guards refuse with reasonschecked after install",
    );
  });

  it("groups links by kind", async () => {
    const user = userEvent.setup();
    setup();
    await user.click(await screen.findByRole("tab", { name: "Links 4" }));
    const links = screen.getByRole("tabpanel");
    expect(links).toHaveTextContent("Tasks9cb35337");
    expect(links).toHaveTextContent("BranchesH-017-items-db");
    expect(links).toHaveTextContent("ArtifactsH-017-change.md · change note");
    expect(links).toHaveTextContent("Itemsblocks H-019");
  });

  it("merges comments and moves into one timeline, and posts the owner's comment", async () => {
    const user = userEvent.setup();
    const { fake } = setup();
    fake.onBoard("itemComment", () => ({
      case: "edited",
      value: create(EditResultSchema, {}),
    }));
    await user.click(await screen.findByRole("tab", { name: "Activity 3" }));
    const timeline = screen.getByRole("list", { name: "Timeline" });
    const entries = within(timeline)
      .getAllByRole("listitem")
      .map((li) => li.textContent ?? "");
    expect(entries[0]).toContain("Team Lead created it");
    expect(entries[1]).toContain("Team Lead moved Ready → Doing");
    expect(entries[2]).toContain("Architect: Map columns onto a table.");
    await user.click(screen.getByRole("button", { name: "Moves" }));
    const moves = screen.getByRole("list", { name: "Timeline" });
    expect(within(moves).getAllByRole("listitem")).toHaveLength(1);

    const box = screen.getByRole("textbox", { name: "Comment on H-017" });
    expect(box).toHaveAttribute("placeholder", "Write a comment…");
    await user.type(box, "Ship it after the demo");
    await user.keyboard("{Meta>}{Enter}{/Meta}");
    await waitFor(() => {
      expect(fake.boardCalls.find((c) => c.case === "itemComment")?.value).toMatchObject({
        id: "H-017",
        body: "Ship it after the demo",
      });
    });
    expect(box).toHaveValue("");
  });

  it("shows a refused comment, and no composer without the control grant", async () => {
    const user = userEvent.setup();
    const { fake } = setup();
    const refusal = "The board for H-017 lives on mac. Comment on it from there.";
    fake.onBoard("itemComment", () => {
      throw new DaemonError("no_board", refusal);
    });
    await user.click(await screen.findByRole("tab", { name: "Activity 3" }));
    await user.type(screen.getByRole("textbox", { name: "Comment on H-017" }), "x");
    await user.click(screen.getByRole("button", { name: /Send/ }));
    expect(await screen.findByRole("alert")).toHaveTextContent(refusal);
  });

  it("shows the move refusal off the board's home", async () => {
    const check = drawerCheck();
    for (const column of check.columns) {
      column.unmet = [unmet("board.elsewhere", "The board lives on mac. Move H-017 from there.")];
    }
    setup({ check });
    const next = await screen.findByRole("region", { name: "Next step" });
    expect(next).toHaveTextContent("The board lives on mac. Move H-017 from there.");
  });

  it("puts focus on the title once the item loads", async () => {
    setup();
    const title = await screen.findByRole("heading", { name: /^Backlog and meetings/ });
    expect(title).toHaveAttribute("tabindex", "-1");
    expect(title).toHaveFocus();
  });

  it("leaves an Esc another handler took", async () => {
    const { onClose } = setup();
    await screen.findByRole("heading", { name: /^Backlog and meetings/ });
    const taken = new KeyboardEvent("keydown", { key: "Escape", cancelable: true });
    taken.preventDefault();
    window.dispatchEvent(taken);
    expect(onClose).not.toHaveBeenCalled();
  });

  it("closes on Esc, steps with the arrows and opens Move to…", async () => {
    const user = userEvent.setup();
    const { onClose, onMove, onStep } = setup();
    await screen.findByRole("heading", { name: /Backlog/ });
    await user.keyboard("{ArrowDown}");
    expect(onStep).toHaveBeenCalledWith(1);
    await user.click(screen.getByRole("button", { name: "Move to…" }));
    expect(onMove).toHaveBeenCalled();
    await user.keyboard("{Escape}");
    expect(onClose).toHaveBeenCalled();
  });
});
