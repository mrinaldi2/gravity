import { describe, expect, it } from "vitest";
import { cardsDaemon } from "../../test/cardFixtures";
import { BATCH, CardCache } from "./cardCache";

describe("CardCache", () => {
  it("asks for a long page in requests the service takes (S2)", async () => {
    const fake = cardsDaemon();
    const cache = new CardCache(fake);
    const ids = Array.from({ length: BATCH * 2 + 5 }, (_, i) => `H-${i + 1}`);
    for (const id of ids) {
      cache.want(id);
    }
    await new Promise((resolve) => setTimeout(resolve, 0));

    const asks = fake.requests
      .map((r) => r.body)
      .flatMap((body) => (body.type === "item_cards_get" ? [body.ids] : []));
    expect(asks.map((batch) => batch.length)).toEqual([BATCH, BATCH, 5]);
    expect(asks.flat()).toEqual(ids);
    // Every id is answered, not just the first request's.
    expect(ids.every((id) => cache.get(id) !== undefined)).toBe(true);
  });
});
