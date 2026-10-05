import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { AddToast } from "../../app/useToasts";
import type { BoardCall } from "../../protocol/board";
import { card, snapshot } from "../../test/boardFixtures";
import { drawerCheck, itemDetail } from "../../test/drawerFixtures";
import { FakeDaemon } from "../../test/fakeDaemon";
import { bot, project } from "../../test/fixtures";
import BoardView from "./BoardView";

const first = card({ id: "H-017", title: "Data model", columnKey: "doing", rank: "a" });
const second = card({ id: "H-018", title: "Board design", columnKey: "doing", rank: "b" });

function idOf(call: BoardCall): string {
  return call.case === "itemGet" ? (call.value.id ?? "") : "";
}

describe("the board's item drawer", () => {
  it("opens from a card, steps through its column and closes on Esc", async () => {
    const user = userEvent.setup();
    const fake = new FakeDaemon()
      .onBoard("boardWatch", () => ({ case: "board", value: snapshot([first, second], 1n) }))
      .onBoard("itemGet", (call) => {
        const detail = itemDetail();
        if (detail.item) {
          detail.item.id = idOf(call);
          detail.item.title = idOf(call) === "H-018" ? "Board design" : "Data model";
        }
        return { case: "item", value: detail };
      })
      .onBoard("itemMoveCheck", () => ({ case: "moveCheck", value: drawerCheck() }));
    render(
      <BoardView
        client={fake}
        project={project()}
        bots={[bot({ id: "dd", name: "Desktop Dev" })]}
        connected
        canControl
        addToast={vi.fn<AddToast>()}
      />,
    );
    await user.click(await screen.findByRole("article", { name: /^H-017/ }));
    const drawer = screen.getByRole("complementary", { name: "Item H-017" });
    // Focus moves into the drawer, on its title (UX-012).
    expect(await within(drawer).findByRole("heading", { name: "Data model" })).toHaveFocus();

    await user.keyboard("{ArrowDown}");
    const next = screen.getByRole("complementary", { name: "Item H-018" });
    expect(await within(next).findByRole("heading", { name: "Board design" })).toHaveFocus();

    await user.click(within(next).getByRole("button", { name: "Move to…" }));
    expect(screen.getByRole("menu")).toBeInTheDocument();
    await user.keyboard("{Escape}");
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("complementary")).toBeNull();
    // Back on the card shown last, not the one that opened the drawer.
    expect(screen.getByRole("article", { name: /^H-018/ })).toHaveFocus();
  });

  it("returns focus to the card when the close button is used", async () => {
    const user = userEvent.setup();
    const fake = new FakeDaemon()
      .onBoard("boardWatch", () => ({ case: "board", value: snapshot([first, second], 1n) }))
      .onBoard("itemGet", () => ({ case: "item", value: itemDetail() }))
      .onBoard("itemMoveCheck", () => ({ case: "moveCheck", value: drawerCheck() }));
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
    const article = await screen.findByRole("article", { name: /^H-017/ });
    article.focus();
    await user.keyboard("{Enter}");
    const drawer = screen.getByRole("complementary", { name: "Item H-017" });
    expect(
      await within(drawer).findByRole("heading", { name: /^Backlog and meetings/ }),
    ).toHaveFocus();
    await user.click(within(drawer).getByRole("button", { name: "Close the item" }));
    expect(screen.queryByRole("complementary")).toBeNull();
    expect(article).toHaveFocus();
  });
});
