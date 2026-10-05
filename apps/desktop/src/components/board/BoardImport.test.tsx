import { create } from "@bufbuild/protobuf";
import { act, render, screen, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { AddToast } from "../../app/useToasts";
import type { ItemCard } from "../../protocol/gen/hermes/board/v1/board_pb";
import { BoardSettingsSchema } from "../../protocol/gen/hermes/board/v1/board_pb";
import { BoardEventKind } from "../../protocol/gen/hermes/board/v1/requests_pb";
import { boardEvent, card, snapshot } from "../../test/boardFixtures";
import { FakeDaemon } from "../../test/fakeDaemon";
import { bot, project } from "../../test/fixtures";
import BoardView from "./BoardView";

// H-101: the backlog import lands 98 items and changes the key from G to H
// while the board is open.
const RANKS = ["V", "k", "s", "w", "y", "z", "zV", "zs", "zw", "zy", "zz", "zzV", "zzk"];
const READY = [...RANKS, "zzs", "zzw", "zzy", "zzz"].map((rank, i) =>
  card({ id: `H-${String(110 + i)}`, title: `Imported ${String(i)}`, columnKey: "ready", rank }),
);

function keyed(cards: readonly ItemCard[], seq: bigint, key: string) {
  const snap = snapshot(cards, seq);
  snap.settings = create(BoardSettingsSchema, { projectId: "p1", key, staleAfterHours: 24 });
  return snap;
}

function burst(fake: FakeDaemon, from: bigint): void {
  fake.emitBoardEvent(boardEvent({ seq: from, kind: BoardEventKind.SETTINGS_CHANGED, itemId: "" }));
  READY.forEach((next, i) => {
    fake.emitBoardEvent(
      boardEvent({
        seq: from + 1n + BigInt(i),
        kind: BoardEventKind.ITEM_UPSERTED,
        itemId: next.id,
        card: next,
      }),
    );
  });
}

function setup(fake: FakeDaemon): void {
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
}

async function readyCards(): Promise<number> {
  const ready = await screen.findByRole("region", { name: /^Ready, 17 items/ });
  return within(ready).getAllByRole("article").length;
}

describe("BoardView after a backlog import (H-101)", () => {
  it("renders all 17 Ready cards when the import's pushes arrive in one burst", async () => {
    const fake = new FakeDaemon()
      .onBoard("boardWatch", () => ({ case: "board", value: keyed([], 0n, "G") }))
      .onBoard("boardGet", () => ({ case: "board", value: keyed(READY, 18n, "H") }));
    setup(fake);
    await screen.findByText("No items yet");
    act(() => {
      burst(fake, 1n);
    });
    expect(await readyCards()).toBe(17);
  });

  it("renders all 17 when the refetch answers before the rest of the burst", async () => {
    const fake = new FakeDaemon()
      .onBoard("boardWatch", () => ({ case: "board", value: keyed([], 0n, "G") }))
      .onBoard("boardGet", () => ({ case: "board", value: keyed([], 1n, "H") }));
    setup(fake);
    await screen.findByText("No items yet");
    await act(async () => {
      fake.emitBoardEvent(
        boardEvent({ seq: 1n, kind: BoardEventKind.SETTINGS_CHANGED, itemId: "" }),
      );
      await Promise.resolve();
    });
    await act(async () => {
      READY.forEach((next, i) => {
        fake.emitBoardEvent(
          boardEvent({
            seq: 2n + BigInt(i),
            kind: BoardEventKind.ITEM_UPSERTED,
            itemId: next.id,
            card: next,
          }),
        );
      });
      await Promise.resolve();
    });
    expect(await readyCards()).toBe(17);
  });
});
