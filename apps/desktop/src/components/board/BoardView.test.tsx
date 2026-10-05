import { create } from "@bufbuild/protobuf";
import { act, fireEvent, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { AddToast } from "../../app/useToasts";
import type { BoardCall } from "../../protocol/board";
import { DaemonError } from "../../protocol/connection";
import type { ItemCard, Unmet } from "../../protocol/gen/hermes/board/v1/board_pb";
import { ItemSchema, Platform } from "../../protocol/gen/hermes/board/v1/board_pb";
import { MoveCheckSchema } from "../../protocol/gen/hermes/board/v1/requests_pb";
import type { MoveCheck } from "../../protocol/gen/hermes/board/v1/requests_pb";
import { card, defaultColumns, moved, snapshot, unmet } from "../../test/boardFixtures";
import { FakeDaemon } from "../../test/fakeDaemon";
import { bot, project } from "../../test/fixtures";
import BoardView from "./BoardView";

const h24 = card({
  id: "H-024",
  title: "Releases tab",
  columnKey: "ready",
  platforms: [Platform.DESKTOP],
  version: 4n,
});
const h17 = card({
  id: "H-017",
  title: "Data model",
  columnKey: "doing",
  assignee: "b1",
  blocked: true,
});

/** Guards per target column for every item; anything unlisted is allowed. */
function checks(byColumn: Readonly<Record<string, readonly Unmet[]>>): (call: BoardCall) => {
  case: "moveCheck";
  value: MoveCheck;
} {
  return (call) => ({
    case: "moveCheck",
    value: create(MoveCheckSchema, {
      itemId: call.case === "itemMoveCheck" ? (call.value.id ?? "") : "",
      columns: defaultColumns().map((c) => ({
        columnKey: c.key,
        unmet: [...(byColumn[c.key] ?? [])],
      })),
    }),
  });
}

function setup(
  cards: readonly ItemCard[] = [h24, h17],
  options: { readonly canControl?: boolean; readonly fake?: FakeDaemon } = {},
) {
  const fake = options.fake ?? new FakeDaemon();
  fake.onBoard("boardWatch", () => ({ case: "board", value: snapshot(cards, 1n) }));
  const addToast = vi.fn<AddToast>();
  render(
    <BoardView
      client={fake}
      project={project()}
      bots={[bot({ id: "b1", name: "Desktop Dev" })]}
      connected
      canControl={options.canControl ?? true}
      addToast={addToast}
    />,
  );
  return { fake, addToast };
}

function columnNamed(name: RegExp): HTMLElement {
  return screen.getByRole("region", { name });
}

describe("BoardView", () => {
  it("renders the columns in order with WIP headers and cards", async () => {
    setup();
    const ready = await screen.findByRole("region", { name: /^Ready, 1 item, limit 10/ });
    expect(
      within(ready).getByRole("article", { name: /^H-024, Feature, Releases tab, Ready/ }),
    ).toBeInTheDocument();
    expect(within(ready).getByText("1/10")).toBeInTheDocument();
    const doing = columnNamed(/^Doing, 1 item, limit 1 per bot, full/);
    expect(within(doing).getByText("Full")).toBeInTheDocument();
    expect(
      within(doing).getByRole("article", {
        name: "H-017, Feature, Data model, Doing, Desktop Dev, blocked",
      }),
    ).toBeInTheDocument();
    expect(screen.queryByRole("region", { name: /^Cancelled/ })).toBeNull();
  });

  it("keeps Inbox a rail until it is opened, and offers hidden columns behind a toggle", async () => {
    const user = userEvent.setup();
    setup([card({ id: "H-1", columnKey: "inbox" })]);
    await user.click(await screen.findByRole("button", { name: /Inbox/ }));
    expect(within(columnNamed(/^Inbox/)).getByRole("article")).toBeInTheDocument();
    await user.click(screen.getByRole("checkbox", { name: "Show cancelled" }));
    expect(columnNamed(/^Cancelled/)).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Collapse Inbox" }));
    expect(screen.queryByRole("article")).toBeNull();
  });

  it("shows an empty state when the project has no items", async () => {
    setup([]);
    expect(await screen.findByText("No items yet")).toBeInTheDocument();
    expect(columnNamed(/^Ready, 0 items/)).toBeInTheDocument();
  });

  it("filters by platform, type and bot, with counts that still read the whole column", async () => {
    const user = userEvent.setup();
    setup([
      h24,
      h17,
      card({ id: "H-030", columnKey: "ready", rank: "a5", platforms: [Platform.IOS] }),
    ]);
    await screen.findByRole("article", { name: /^H-024/ });

    await user.click(screen.getByRole("button", { name: "ios" }));
    expect(screen.queryByRole("article", { name: /^H-024/ })).toBeNull();
    expect(screen.getByRole("article", { name: /^H-030/ })).toBeInTheDocument();
    expect(
      within(columnNamed(/^Ready, 2 items, 1 shown/)).getByText("1 of 2 · limit 10"),
    ).toBeInTheDocument();
    expect(screen.getByRole("status", { name: "" })).toHaveTextContent("Showing 1 of 3");

    await user.click(screen.getByRole("button", { name: "Clear" }));
    expect(screen.getAllByRole("article")).toHaveLength(3);

    await user.click(screen.getByRole("button", { name: "Bot" }));
    await user.click(screen.getByRole("checkbox", { name: "Desktop Dev" }));
    expect(screen.getAllByRole("article").map((a) => a.dataset["itemId"])).toEqual(["H-017"]);

    await user.click(screen.getByRole("button", { name: "Bot · 1" }));
    await user.click(screen.getByRole("button", { name: "✱ Bug" }));
    expect(screen.queryAllByRole("article")).toHaveLength(0);
  });

  it("moves a card when a push arrives", async () => {
    const { fake } = setup();
    await screen.findByRole("article", { name: /^H-024/ });
    act(() => {
      fake.emitBoardEvent(moved(2n, card({ ...h24, columnKey: "review" }), "ready"));
    });
    expect(
      within(columnNamed(/^Review/)).getByRole("article", { name: /^H-024/ }),
    ).toBeInTheDocument();
    expect(within(columnNamed(/^Ready/)).queryByRole("article")).toBeNull();
  });

  it("lists every column in Move to…, and shows a refusal with every unmet guard", async () => {
    const user = userEvent.setup();
    const fake = new FakeDaemon().onBoard(
      "itemMoveCheck",
      checks({
        review: [
          unmet("review.branch", "Link a branch first.", "Link the branch."),
          unmet("review.note", "Link a change note."),
        ],
      }),
    );
    setup(undefined, { fake });
    const article = await screen.findByRole("article", { name: /^H-024/ });
    article.focus();
    await user.keyboard("m");

    const menu = await screen.findByRole("menu", { name: "Move H-024 to" });
    const items = within(menu).getAllByRole("menuitem");
    expect(items.map((item) => item.getAttribute("aria-label"))).toEqual([
      "Inbox",
      "Doing",
      "Review, can't move: Link a branch first. +1 more",
      "Verify",
      "Awaiting owner",
      "Deploying",
      "Done",
      "Cancelled",
    ]);
    expect(items[0]).toHaveFocus();
    await user.keyboard("{ArrowDown}{ArrowDown}{Enter}");

    const refusal = await screen.findByRole("dialog", { name: "Can't move H-024 to Review" });
    expect(refusal).toHaveTextContent("Link a branch first.");
    expect(refusal).toHaveTextContent("Link the branch.");
    expect(refusal).toHaveTextContent("Link a change note.");
    expect(fake.boardCalls.some((call) => call.case === "itemMove")).toBe(false);

    await user.keyboard("{Escape}");
    expect(screen.queryByRole("dialog")).toBeNull();
    await vi.waitFor(() => {
      expect(article).toHaveFocus();
    });
  });

  it("moves an allowed item with its version and says so", async () => {
    const user = userEvent.setup();
    const fake = new FakeDaemon().onBoard("itemMoveCheck", checks({})).onBoard("itemMove", () => ({
      case: "moved",
      value: {
        $typeName: "hermes.board.v1.MoveResult",
        outcome: { case: "done", value: create(ItemSchema, { id: "H-024" }) },
      },
    }));
    const { addToast } = setup(undefined, { fake });
    await user.click(await screen.findByRole("button", { name: "Move H-024 to…" }));
    await user.click(await screen.findByRole("menuitem", { name: "Review" }));
    await vi.waitFor(() => {
      expect(addToast).toHaveBeenCalledWith("info", "H-024 → Review", "");
    });
    expect(fake.boardCalls.at(-1)).toEqual({
      case: "itemMove",
      value: {
        id: "H-024",
        to: "review",
        expectedVersion: 4n,
        reason: undefined,
        overrideReason: undefined,
      },
    });
  });

  it("asks for an override reason when the target is at its WIP limit", async () => {
    const user = userEvent.setup();
    const fake = new FakeDaemon()
      .onBoard(
        "itemMoveCheck",
        checks({ doing: [unmet("wip.full", "Doing is at its WIP limit of 1.")] }),
      )
      .onBoard("itemMove", () => ({
        case: "moved",
        value: {
          $typeName: "hermes.board.v1.MoveResult",
          outcome: { case: "done", value: create(ItemSchema, { id: "H-024" }) },
        },
      }));
    setup(undefined, { fake });
    (await screen.findByRole("article", { name: /^H-024/ })).focus();
    await user.keyboard("m");
    await user.click(await screen.findByRole("menuitem", { name: "Doing" }));

    const dialog = await screen.findByRole("dialog", { name: "Move H-024 to Doing" });
    expect(dialog).toHaveTextContent("Doing is at its WIP limit of 1.");
    const go = within(dialog).getByRole("button", { name: "Move anyway" });
    expect(go).toBeDisabled();
    await user.type(within(dialog).getByRole("textbox", { name: /Override reason/ }), "hotfix");
    await user.click(go);
    await vi.waitFor(() => {
      expect(fake.boardCalls.at(-1)).toMatchObject({
        case: "itemMove",
        value: { to: "doing", overrideReason: "hotfix" },
      });
    });
  });

  it("lets the owner close a Verify item without a release, with a reason", async () => {
    const user = userEvent.setup();
    const verify = card({ id: "H-058", title: "First desktop release", columnKey: "verify" });
    const fake = new FakeDaemon()
      .onBoard(
        "itemMoveCheck",
        checks({ done: [unmet("reason.required", "It skips the release; say why it is done.")] }),
      )
      .onBoard("itemMove", () => ({
        case: "moved",
        value: {
          $typeName: "hermes.board.v1.MoveResult",
          outcome: { case: "done", value: create(ItemSchema, { id: "H-058" }) },
        },
      }));
    setup([verify], { fake });
    await user.click(await screen.findByRole("button", { name: "Move H-058 to…" }));
    await user.click(await screen.findByRole("menuitem", { name: "Done" }));

    const dialog = await screen.findByRole("dialog", { name: "Move H-058 to Done" });
    expect(dialog).toHaveTextContent("It skips the release; say why it is done.");
    const go = within(dialog).getByRole("button", { name: "Move" });
    expect(go).toBeDisabled();
    await user.type(within(dialog).getByRole("textbox", { name: /Reason/ }), "shipped in 0.14.0");
    await user.click(go);
    await vi.waitFor(() => {
      expect(fake.boardCalls.at(-1)).toMatchObject({
        case: "itemMove",
        value: { id: "H-058", to: "done", reason: "shipped in 0.14.0", overrideReason: undefined },
      });
    });
  });

  it("refreshes the board when a move finds it stale", async () => {
    const user = userEvent.setup();
    const fake = new FakeDaemon()
      .onBoard("itemMoveCheck", checks({}))
      .onBoard("boardGet", () => ({ case: "board", value: snapshot([h24], 9n) }))
      .onBoard("itemMove", () => ({
        case: "moved",
        value: {
          $typeName: "hermes.board.v1.MoveResult",
          outcome: { case: "conflict", value: create(ItemSchema, { id: "H-024" }) },
        },
      }));
    const { addToast } = setup(undefined, { fake });
    await user.click(await screen.findByRole("button", { name: "Move H-024 to…" }));
    await user.click(await screen.findByRole("menuitem", { name: "Done" }));
    await vi.waitFor(() => {
      expect(addToast).toHaveBeenCalledWith("warn", "H-024 changed meanwhile", expect.any(String));
    });
    expect(fake.boardCalls.at(-1)?.case).toBe("boardGet");
  });

  it("dims refused columns with a reason chip while dragging, and explains a refused drop", async () => {
    const fake = new FakeDaemon().onBoard(
      "itemMoveCheck",
      checks({ deploying: [unmet("move.daemon_only", "Moves after the release ruling.")] }),
    );
    setup(undefined, { fake });
    const article = await screen.findByRole("article", { name: /^H-024/ });
    fireEvent.dragStart(article, {
      dataTransfer: { setData: vi.fn<(format: string, data: string) => void>(), effectAllowed: "" },
    });
    const deploying = columnNamed(/^Deploying/);
    await vi.waitFor(() => {
      expect(deploying).toHaveClass("board-column-refused");
    });
    expect(within(deploying).getByText("Moves after the release ruling.")).toBeInTheDocument();
    expect(columnNamed(/^Review/)).toHaveClass("board-column-allowed");

    fireEvent.dragEnter(deploying);
    expect(screen.getByText("Deploying: Moves after the release ruling.")).toBeInTheDocument();
    fireEvent.drop(deploying, { clientY: 100 });
    expect(
      await screen.findByRole("dialog", { name: "Can't move H-024 to Deploying" }),
    ).toBeInTheDocument();
    expect(deploying).not.toHaveClass("board-column-refused");
  });

  it("offers no moves on a read-only connection", async () => {
    setup(undefined, { canControl: false });
    const article = await screen.findByRole("article", { name: /^H-024/ });
    expect(screen.queryByRole("button", { name: /^Move / })).toBeNull();
    expect(article).toHaveAttribute("draggable", "false");
  });

  it("explains a board that isn't set up yet, and an older Hermes service", async () => {
    const fake = new FakeDaemon().onBoard("boardWatch", () => {
      throw new DaemonError("no_board", "no board");
    });
    render(
      <BoardView
        client={fake}
        project={project()}
        bots={[]}
        connected
        canControl={false}
        addToast={vi.fn<AddToast>()}
      />,
    );
    expect(
      await screen.findByRole("heading", { name: "This board isn't set up yet" }),
    ).toBeInTheDocument();

    const old = new FakeDaemon();
    old.encodings = [];
    render(
      <BoardView
        client={old}
        project={project()}
        bots={[]}
        connected
        canControl
        addToast={vi.fn<AddToast>()}
      />,
    );
    expect(
      screen.getByRole("heading", { name: "The board needs a newer Hermes service" }),
    ).toBeInTheDocument();
  });

  it("offers a retry when the board fails to load", async () => {
    const user = userEvent.setup();
    let fail = true;
    const fake = new FakeDaemon().onBoard("boardWatch", () => {
      if (fail) {
        throw new DaemonError("internal", "database is locked");
      }
      return { case: "board", value: snapshot([h24], 0n) };
    });
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
    expect(await screen.findByText("database is locked")).toBeInTheDocument();
    fail = false;
    await user.click(screen.getByRole("button", { name: "Try again" }));
    expect(await screen.findByRole("article", { name: /^H-024/ })).toBeInTheDocument();
  });
});
