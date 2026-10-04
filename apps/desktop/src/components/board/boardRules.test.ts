import { create } from "@bufbuild/protobuf";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ItemType, Platform } from "../../protocol/gen/hermes/board/v1/board_pb";
import { MoveCheckSchema } from "../../protocol/gen/hermes/board/v1/requests_pb";
import { card, column, unmet } from "../../test/boardFixtures";
import { loadBoardPrefs, saveBoardPrefs } from "./boardPrefs";
import { isFiltering, matchesFilters, NO_FILTERS, parseFilters, toggled } from "./filters";
import { CHECK_TTL_MS, columnChecks, MoveCheckCache, planMove, reasonChip } from "./moves";
import { countText, wipDescription, wipSummary } from "./wip";

describe("wipSummary", () => {
  const review = column("review"); // limit 3
  const three = [card({ id: "a" }), card({ id: "b" }), card({ id: "c" })];

  it("reads under, full and over against a column limit", () => {
    expect(wipSummary(review, three.slice(0, 2), 2).state).toBe("under");
    expect(wipSummary(review, three, 3).state).toBe("full");
    const over = wipSummary(review, [...three, card({ id: "d" })], 4);
    expect(over.state).toBe("over");
    expect(wipDescription("Review", over)).toBe("Review, 4 items, limit 3, over limit by 1");
  });

  it("has no state without a limit", () => {
    const summary = wipSummary(column("done"), three, 3);
    expect(summary.state).toBe("none");
    expect(wipDescription("Done", summary)).toBe("Done, 3 items");
  });

  it("counts per assignee for a per-bot limit, busiest first", () => {
    const doing = column("doing"); // 1 per bot
    const summary = wipSummary(
      doing,
      [
        card({ id: "a", assignee: "b1" }),
        card({ id: "b", assignee: "b2" }),
        card({ id: "c", assignee: "b2" }),
      ],
      3,
    );
    expect(summary.loads).toEqual([
      { botId: "b2", count: 2 },
      { botId: "b1", count: 1 },
    ]);
    expect(summary.state).toBe("over");
    expect(wipDescription("Doing", summary)).toBe(
      "Doing, 3 items, limit 1 per bot, a bot is over its limit",
    );
  });

  it("says '2 of 5' while filters hide cards", () => {
    const summary = wipSummary(column("done"), [...three, card({ id: "d" }), card({ id: "e" })], 2);
    expect(countText(summary)).toBe("2 of 5");
    expect(wipDescription("Done", summary)).toBe("Done, 5 items, 2 shown");
  });
});

describe("filters", () => {
  const item = card({
    assignee: "b1",
    type: ItemType.BUG,
    platforms: [Platform.IOS, Platform.DAEMON],
  });

  it("passes everything with no filters", () => {
    expect(isFiltering(NO_FILTERS)).toBe(false);
    expect(matchesFilters(item, NO_FILTERS)).toBe(true);
  });

  it("requires every set filter, with platforms matching on any overlap", () => {
    expect(matchesFilters(item, { ...NO_FILTERS, bots: ["b1"] })).toBe(true);
    expect(matchesFilters(item, { ...NO_FILTERS, bots: ["b2"] })).toBe(false);
    expect(matchesFilters(card(), { ...NO_FILTERS, bots: ["b1"] })).toBe(false);
    expect(
      matchesFilters(item, { ...NO_FILTERS, platforms: [Platform.DESKTOP, Platform.IOS] }),
    ).toBe(true);
    expect(matchesFilters(item, { ...NO_FILTERS, platforms: [Platform.DESKTOP] })).toBe(false);
    expect(matchesFilters(item, { bots: ["b1"], platforms: [], types: [ItemType.FEATURE] })).toBe(
      false,
    );
  });

  it("toggles values in and out", () => {
    expect(toggled(["a"], "b")).toEqual(["a", "b"]);
    expect(toggled(["a", "b"], "a")).toEqual(["b"]);
  });

  it("drops anything unknown when reading stored filters back", () => {
    expect(parseFilters({ bots: ["b1", 3], platforms: [1, 0, 42, "x"], types: [3, 99] })).toEqual({
      bots: ["b1"],
      platforms: [Platform.DESKTOP],
      types: [ItemType.BUG],
    });
    expect(parseFilters("nonsense")).toEqual(NO_FILTERS);
  });
});

describe("board prefs", () => {
  let store: Map<string, string>;

  beforeEach(() => {
    store = new Map();
    vi.stubGlobal("localStorage", {
      getItem: (key: string): string | null => store.get(key) ?? null,
      setItem: (key: string, value: string): void => {
        store.set(key, value);
      },
    });
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("remembers filters, Inbox and hidden columns per project", () => {
    expect(loadBoardPrefs("p1")).toEqual({
      filters: NO_FILTERS,
      inboxOpen: false,
      showHidden: false,
    });
    saveBoardPrefs("p1", {
      filters: { ...NO_FILTERS, bots: ["b1"] },
      inboxOpen: true,
      showHidden: true,
    });
    expect(loadBoardPrefs("p1")).toEqual({
      filters: { ...NO_FILTERS, bots: ["b1"] },
      inboxOpen: true,
      showHidden: true,
    });
    expect(loadBoardPrefs("p2").inboxOpen).toBe(false);
  });

  it("falls back to defaults for unreadable storage", () => {
    store.set("hermes.board.p1", "{nope");
    expect(loadBoardPrefs("p1").inboxOpen).toBe(false);
  });
});

describe("planMove", () => {
  it("goes when nothing is unmet", () => {
    expect(planMove([])).toEqual({ kind: "go" });
  });

  it("asks for a reason or a WIP override when those are all that is missing", () => {
    expect(planMove([unmet("reason.required", "Say why.")])).toMatchObject({
      kind: "input",
      needsReason: true,
      needsOverride: false,
    });
    expect(planMove([unmet("wip.full", "Doing is full.")])).toMatchObject({
      kind: "input",
      needsReason: false,
      needsOverride: true,
    });
  });

  it("refuses when any other guard is unmet", () => {
    const refused = planMove([
      unmet("reason.required", "Say why."),
      unmet("role.not_allowed", "Only the lead."),
    ]);
    expect(refused.kind).toBe("refused");
  });

  it("chips the first reason plus how many more", () => {
    expect(reasonChip([unmet("a", "Link a branch."), unmet("b", "x"), unmet("c", "y")])).toBe(
      "Link a branch. +2 more",
    );
    expect(reasonChip([unmet("a", "Link a branch.")])).toBe("Link a branch.");
  });
});

describe("MoveCheckCache", () => {
  const checks = columnChecks(
    create(MoveCheckSchema, { itemId: "H-1", columns: [{ columnKey: "ready", unmet: [] }] }),
  );

  it("serves a check for the same version within the TTL", () => {
    const cache = new MoveCheckCache();
    cache.set("H-1", 2n, 1000, checks);
    expect(cache.get("H-1", 2n, 1000 + CHECK_TTL_MS)).toBe(checks);
    expect(cache.get("H-1", 2n, 1001 + CHECK_TTL_MS)).toBeUndefined();
    expect(cache.get("H-1", 3n, 1000)).toBeUndefined();
    cache.clear();
    expect(cache.get("H-1", 2n, 1000)).toBeUndefined();
  });
});
