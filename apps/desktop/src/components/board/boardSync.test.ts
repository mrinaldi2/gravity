import { describe, expect, it, vi } from "vitest";
import { DaemonError } from "../../protocol/connection";
import { BoardEventKind } from "../../protocol/gen/hermes/board/v1/requests_pb";
import type { BoardSnapshot } from "../../protocol/gen/hermes/board/v1/requests_pb";
import { boardEvent, card, moved, snapshot } from "../../test/boardFixtures";
import { FakeDaemon } from "../../test/fakeDaemon";
import type { BoardModel, BoardStatus, EventOutcome } from "./boardSync";
import { applyEvent, BoardSync, fromSnapshot } from "./boardSync";

const h1 = card({ id: "H-1", columnKey: "ready" });
const h2 = card({ id: "H-2", columnKey: "doing" });

function model(seq = 3n): BoardModel {
  return fromSnapshot(snapshot([h1, h2], seq));
}

function applied(outcome: EventOutcome): BoardModel {
  if (outcome.kind !== "applied") {
    throw new Error(`expected the push to apply, got ${outcome.kind}`);
  }
  return outcome.model;
}

function columnOf(m: BoardModel, id: string): string | undefined {
  return m.cards.get(id)?.columnKey;
}

describe("applyEvent", () => {
  it("applies the next seq and advances to it", () => {
    const outcome = applyEvent(
      model(),
      moved(4n, card({ id: "H-1", columnKey: "doing" }), "ready"),
    );
    expect(applied(outcome).seq).toBe(4n);
    expect(columnOf(applied(outcome), "H-1")).toBe("doing");
  });

  it("ignores a push the snapshot already holds", () => {
    expect(applyEvent(model(), moved(3n, h1, "ready")).kind).toBe("ignored");
    expect(applyEvent(model(), moved(1n, h1, "ready")).kind).toBe("ignored");
  });

  it("asks for a resync on a gap", () => {
    expect(applyEvent(model(), moved(5n, h1, "ready")).kind).toBe("resync");
  });

  it("reads a restarted daemon's low seq as a gap once it is past the snapshot", () => {
    // Seq lives in memory, so a restart starts again from 1.
    expect(applyEvent(model(0n), moved(1n, h1, "ready")).kind).toBe("applied");
    expect(applyEvent(model(7n), moved(9n, h1, "ready")).kind).toBe("resync");
  });

  it("inserts upserted cards and drops removed ones", () => {
    const added = applyEvent(
      model(),
      boardEvent({
        seq: 4n,
        kind: BoardEventKind.ITEM_UPSERTED,
        itemId: "H-3",
        card: card({ id: "H-3" }),
      }),
    );
    expect(applied(added).cards.has("H-3")).toBe(true);
    const removed = applyEvent(
      model(),
      boardEvent({ seq: 4n, kind: BoardEventKind.ITEM_REMOVED, itemId: "H-1" }),
    );
    expect(applied(removed).cards.has("H-1")).toBe(false);
  });

  it.each([BoardEventKind.COLUMNS_CHANGED, BoardEventKind.SETTINGS_CHANGED, BoardEventKind.RESYNC])(
    "refetches on event kind %s",
    (kind) => {
      expect(applyEvent(model(), boardEvent({ seq: 4n, kind, itemId: "" })).kind).toBe("resync");
    },
  );

  it("refetches when a card push carries no card", () => {
    expect(
      applyEvent(model(), boardEvent({ seq: 4n, kind: BoardEventKind.ITEM_MOVED, itemId: "H-1" }))
        .kind,
    ).toBe("resync");
  });

  it("keeps an unknown kind's place in the sequence", () => {
    const outcome = applyEvent(
      model(),
      boardEvent({ seq: 4n, kind: Number.parseInt("99", 10), itemId: "" }),
    );
    expect(applied(outcome).seq).toBe(4n);
  });

  it("orders columns by ord whatever order the snapshot sends", () => {
    const snap = snapshot();
    snap.columns.reverse();
    expect(fromSnapshot(snap).columns.map((c) => c.key)[0]).toBe("inbox");
  });
});

/** A started sync over a fake daemon, with every status it reported. */
function startSync(fake: FakeDaemon): {
  sync: BoardSync;
  seen: BoardStatus[];
  latest: () => BoardModel;
} {
  const seen: BoardStatus[] = [];
  const sync = new BoardSync(fake, "p1", (status) => {
    seen.push(status);
  });
  sync.start();
  return {
    sync,
    seen,
    latest: () => {
      const last = seen.at(-1);
      if (last?.kind !== "ready") {
        throw new Error(`board is ${last?.kind ?? "unset"}`);
      }
      return last.model;
    },
  };
}

function reply(snap: BoardSnapshot): { case: "board"; value: BoardSnapshot } {
  return { case: "board", value: snap };
}

describe("BoardSync", () => {
  it("watches the project and applies pushes in order", async () => {
    const fake = new FakeDaemon().onBoard("boardWatch", () => reply(snapshot([h1], 2n)));
    const { latest } = startSync(fake);
    await vi.waitFor(() => {
      expect(latest().seq).toBe(2n);
    });
    expect(fake.boardCalls[0]).toEqual({ case: "boardWatch", value: { projectId: "p1" } });

    fake.emitBoardEvent(moved(3n, card({ id: "H-1", columnKey: "doing" }), "ready"));
    expect(columnOf(latest(), "H-1")).toBe("doing");
    expect(latest().seq).toBe(3n);
  });

  it("ignores pushes for other projects", async () => {
    const fake = new FakeDaemon().onBoard("boardWatch", () => reply(snapshot([h1], 0n)));
    const { latest } = startSync(fake);
    await vi.waitFor(() => {
      expect(latest().seq).toBe(0n);
    });
    fake.emitBoardEvent(
      boardEvent({ projectId: "p2", seq: 1n, kind: BoardEventKind.ITEM_REMOVED, itemId: "H-1" }),
    );
    expect(latest().cards.has("H-1")).toBe(true);
  });

  it("refetches on a seq gap and continues from the new snapshot", async () => {
    const fake = new FakeDaemon()
      .onBoard("boardWatch", () => reply(snapshot([h1], 1n)))
      .onBoard("boardGet", () => reply(snapshot([card({ id: "H-1", columnKey: "review" })], 5n)));
    const { latest } = startSync(fake);
    await vi.waitFor(() => {
      expect(latest().seq).toBe(1n);
    });

    // seq 2 and 3 were lost: 4 is a gap.
    fake.emitBoardEvent(moved(4n, card({ id: "H-1", columnKey: "verify" }), "review"));
    await vi.waitFor(() => {
      expect(latest().seq).toBe(5n);
    });
    expect(fake.boardCalls.map((call) => call.case)).toEqual(["boardWatch", "boardGet"]);
    expect(columnOf(latest(), "H-1")).toBe("review");

    fake.emitBoardEvent(moved(6n, card({ id: "H-1", columnKey: "done" }), "review"));
    expect(columnOf(latest(), "H-1")).toBe("done");
  });

  it("holds pushes that arrive during a refetch and replays the newer ones", async () => {
    const held: { answer?: () => void } = {};
    const fake = new FakeDaemon()
      .onBoard("boardWatch", () => reply(snapshot([h1, h2], 1n)))
      .onBoard(
        "boardGet",
        () =>
          new Promise((resolve) => {
            held.answer = () => {
              resolve(reply(snapshot([card({ id: "H-1", columnKey: "review" }), h2], 5n)));
            };
          }),
      );
    const { latest } = startSync(fake);
    await vi.waitFor(() => {
      expect(latest().seq).toBe(1n);
    });

    fake.emitBoardEvent(moved(3n, h1, "ready")); // a gap: starts the refetch
    await vi.waitFor(() => {
      expect(held.answer).toBeDefined();
    });
    fake.emitBoardEvent(moved(5n, card({ id: "H-1", columnKey: "stale" }), "x")); // in the snapshot
    fake.emitBoardEvent(moved(6n, card({ id: "H-2", columnKey: "done" }), "doing")); // after it
    held.answer?.();

    await vi.waitFor(() => {
      expect(latest().seq).toBe(6n);
    });
    expect(columnOf(latest(), "H-1")).toBe("review");
    expect(columnOf(latest(), "H-2")).toBe("done");
    expect(fake.boardCalls.filter((call) => call.case === "boardGet")).toHaveLength(1);
  });

  it("refetches on a RESYNC push", async () => {
    const fake = new FakeDaemon()
      .onBoard("boardWatch", () => reply(snapshot([], 1n)))
      .onBoard("boardGet", () => reply(snapshot([h1], 2000n)));
    const { latest } = startSync(fake);
    await vi.waitFor(() => {
      expect(latest().seq).toBe(1n);
    });
    fake.emitBoardEvent(boardEvent({ seq: 2n, kind: BoardEventKind.RESYNC, itemId: "" }));
    await vi.waitFor(() => {
      expect(latest().seq).toBe(2000n);
    });
  });

  it("reports a daemon error's code, and retries the watch on refresh", async () => {
    let enabled = false;
    const fake = new FakeDaemon().onBoard("boardWatch", () => {
      if (!enabled) {
        throw new DaemonError("no_board", "the board is not enabled");
      }
      return reply(snapshot([h1], 0n));
    });
    const { sync, seen, latest } = startSync(fake);
    await vi.waitFor(() => {
      expect(seen.at(-1)).toEqual({
        kind: "failed",
        code: "no_board",
        message: "the board is not enabled",
      });
    });
    enabled = true;
    sync.refresh();
    await vi.waitFor(() => {
      expect(latest().cards.has("H-1")).toBe(true);
    });
  });

  it("unwatches on stop and ignores later pushes", async () => {
    const fake = new FakeDaemon()
      .onBoard("boardWatch", () => reply(snapshot([h1], 0n)))
      .onBoard("boardUnwatch", () => ({
        case: "unwatched",
        value: { $typeName: "hermes.board.v1.BoardUnwatched", projectId: "p1" },
      }));
    const { sync, seen } = startSync(fake);
    await vi.waitFor(() => {
      expect(seen).toHaveLength(1);
    });
    sync.stop();
    fake.emitBoardEvent(moved(1n, h1, "ready"));
    expect(seen).toHaveLength(1);
    expect(fake.boardCalls.at(-1)).toEqual({ case: "boardUnwatch", value: { projectId: "p1" } });
  });
});
