// Card ids as links (H-203): a service that answers `item_cards_get` and the
// board reads the card drawer makes, for tests and stories.

import { toJson } from "@bufbuild/protobuf";
import type { Bot, Project } from "../protocol/entities";
import { ItemCardSchema, ItemType, Priority } from "../protocol/gen/hermes/board/v1/board_pb";
import type { ItemCardEntry } from "../protocol/itemCards";
import { card } from "./boardFixtures";
import { itemDetail } from "./drawerFixtures";
import { FakeDaemon } from "./fakeDaemon";
import { bot, project } from "./fixtures";

export const CARD_BOTS: readonly Bot[] = [bot({ id: "dd", name: "Desktop Dev" })];
export const CARD_PROJECT: Project = project({ id: "p1", name: "The Hermes", item_prefix: "H" });

function entry(id: string, title: string): ItemCardEntry {
  const json = toJson(
    ItemCardSchema,
    card({
      id,
      title,
      type: ItemType.BUG,
      priority: Priority.P0,
      assignee: "dd",
      columnKey: "doing",
    }),
  );
  return {
    id,
    project_id: "p1",
    project_name: "The Hermes",
    computer: "mac",
    card: json as ItemCardEntry["card"],
    column_name: "Doing",
  };
}

const ENTRIES: Readonly<Record<string, ItemCardEntry>> = {
  "H-293": entry("H-293", "Clicks open the wrong bot"),
  "H-292": entry("H-292", "Card ids as links"),
  "H-999": { id: "H-999", project_id: "p1", project_name: "The Hermes", missing: true },
  "H-500": {
    ...entry("H-500", "Kept elsewhere"),
    unreachable: { computer: "win-pc", last_seen: "2026-10-08T13:30:00Z" },
  },
};

/** H-293 and H-292 on the board, H-999 deleted, H-500 kept on an offline win-pc. */
export function cardsDaemon(): FakeDaemon {
  const fake = new FakeDaemon();
  fake.capabilities = [...fake.capabilities, "item_cards"];
  fake.onRequest("item_cards_get", (body) => ({
    type: "item_cards",
    req_id: "1",
    cards: (body.type === "item_cards_get" ? body.ids : []).map(
      (id) => ENTRIES[id] ?? { id, missing: true },
    ),
  }));
  fake.onBoard("boardGet", () => {
    throw new Error("no board here");
  });
  fake.onBoard("itemMoveCheck", () => {
    throw new Error("no check here");
  });
  fake.onBoard("itemGet", (call) => {
    const id: string = (call.case === "itemGet" ? call.value.id : undefined) ?? "";
    const detail = itemDetail();
    if (detail.item) {
      detail.item.id = id;
      detail.item.title = ENTRIES[id]?.card?.title ?? id;
      detail.item.description = id === "H-293" ? "Same as H-292." : "";
    }
    return { case: "item", value: detail };
  });
  return fake;
}
