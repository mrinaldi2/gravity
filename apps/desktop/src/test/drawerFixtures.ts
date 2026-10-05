// An item as the drawer reads it (U4): detail with criteria, people, links,
// comments and history, and a guard check for its next move.

import { create } from "@bufbuild/protobuf";
import { timestampFromDate } from "@bufbuild/protobuf/wkt";
import {
  ColumnCategory,
  ItemEventKind,
  ItemSchema,
  ItemType,
  LinkKind,
  PersonRole,
  Platform,
  Priority,
  Size,
  VerificationResult,
} from "../protocol/gen/hermes/board/v1/board_pb";
import { ItemDetailSchema, MoveCheckSchema } from "../protocol/gen/hermes/board/v1/requests_pb";
import type { ItemDetail, MoveCheck } from "../protocol/gen/hermes/board/v1/requests_pb";
import { defaultColumns, unmet } from "./boardFixtures";

const at = (iso: string) => timestampFromDate(new Date(iso));

/** "Now" for the drawer's ages: 31 hours after H-017 entered Doing. */
export const DRAWER_NOW = Date.parse("2026-10-05T17:00:00Z");

export function itemDetail(): ItemDetail {
  return create(ItemDetailSchema, {
    item: create(ItemSchema, {
      id: "H-017",
      seq: 17,
      type: ItemType.FEATURE,
      title: "Backlog and meetings data model in the DB",
      description: "## Why\n\nThe board needs a home.\n\n## Done when\n\nItems survive a restart.",
      platforms: [Platform.DESKTOP, Platform.DAEMON],
      size: Size.M,
      priority: Priority.P1,
      columnKey: "doing",
      category: ColumnCategory.DOING,
      assignee: "dd",
      parentId: "H-016",
      acceptanceCriteria: [
        {
          idx: 0,
          text: "Items survive a restart",
          checked: true,
          checkedBy: "bot:tw",
          checkedAt: at("2026-10-05T14:00:00Z"),
          machine: "mac",
        },
        { idx: 1, text: "Guards refuse with reasons", checked: false },
      ],
      people: [{ botId: "arch", role: PersonRole.REVIEWER }],
      verifications: [{ machine: "mac", result: VerificationResult.PASS, by: "tw" }],
      createdBy: "bot:lead",
      stateEnteredAt: at("2026-10-04T10:00:00Z"),
      version: 7n,
    }),
    links: [
      { itemId: "H-017", kind: LinkKind.TASK, ref: "9cb35337-aaaa-bbbb", createdBy: "bot:lead" },
      { itemId: "H-017", kind: LinkKind.BRANCH, ref: "H-017-items-db", createdBy: "bot:dd" },
      {
        itemId: "H-017",
        kind: LinkKind.ARTIFACT,
        ref: "H-017-change.md",
        label: "change note",
        createdBy: "bot:dd",
      },
      { itemId: "H-017", kind: LinkKind.ITEM_BLOCKS, ref: "H-019", createdBy: "bot:lead" },
    ],
    comments: [
      {
        id: "c1",
        itemId: "H-017",
        author: "bot:arch",
        body: "Map columns onto a table.",
        at: at("2026-10-05T09:40:00Z"),
      },
    ],
    history: [
      {
        id: 1n,
        itemId: "H-017",
        actor: "bot:lead",
        kind: ItemEventKind.CREATED,
        at: at("2026-10-03T08:00:00Z"),
      },
      {
        id: 2n,
        itemId: "H-017",
        actor: "bot:lead",
        kind: ItemEventKind.MOVED,
        from: "ready",
        to: "doing",
        at: at("2026-10-04T10:00:00Z"),
      },
    ],
  });
}

/** What stands between H-017 and Review: its branch is linked, its note isn't. */
export function drawerCheck(): MoveCheck {
  return create(MoveCheckSchema, {
    itemId: "H-017",
    columns: defaultColumns().map((c) => ({
      columnKey: c.key,
      unmet:
        c.key === "review"
          ? [
              unmet(
                "review.change_note",
                "No change note is linked.",
                "Link the change-note artifact.",
              ),
            ]
          : [],
    })),
  });
}
