import { create } from "@bufbuild/protobuf";
import type { MessageInitShape } from "@bufbuild/protobuf";
import type { BoardColumn, ItemCard, Unmet } from "../protocol/gen/hermes/board/v1/board_pb";
import {
  BoardColumnSchema,
  BoardSettingsSchema,
  ColumnCategory,
  ItemCardSchema,
  ItemType,
  Platform,
  Priority,
  UnmetSchema,
  WipScope,
} from "../protocol/gen/hermes/board/v1/board_pb";
import type { BoardEvent, BoardSnapshot } from "../protocol/gen/hermes/board/v1/requests_pb";
import {
  BoardEventKind,
  BoardEventSchema,
  BoardSnapshotSchema,
} from "../protocol/gen/hermes/board/v1/requests_pb";

const PROJECT = "p1";

/** The B2 default columns: Inbox to Done, with Cancelled hidden. */
export function defaultColumns(): BoardColumn[] {
  const spec: [string, string, ColumnCategory, number | undefined, WipScope?, boolean?][] = [
    ["inbox", "Inbox", ColumnCategory.INBOX, undefined],
    ["ready", "Ready", ColumnCategory.READY, 10],
    ["doing", "Doing", ColumnCategory.DOING, 1, WipScope.PER_ASSIGNEE],
    ["review", "Review", ColumnCategory.REVIEW, 3],
    ["verify", "Verify", ColumnCategory.VERIFY, 3],
    ["awaiting", "Awaiting owner", ColumnCategory.APPROVAL, 5],
    ["deploying", "Deploying", ColumnCategory.DEPLOYING, undefined],
    ["done", "Done", ColumnCategory.DONE, undefined],
    ["cancelled", "Cancelled", ColumnCategory.CANCELLED, undefined, undefined, false],
  ];
  return spec.map(([key, name, category, wipLimit, wipScope, visible], ord) =>
    create(BoardColumnSchema, {
      projectId: PROJECT,
      key,
      name,
      ord,
      category,
      wipLimit,
      wipScope: wipScope ?? (wipLimit === undefined ? WipScope.UNSPECIFIED : WipScope.COLUMN),
      visible: visible ?? true,
    }),
  );
}

export function card(over: MessageInitShape<typeof ItemCardSchema> = {}): ItemCard {
  return create(ItemCardSchema, {
    id: "H-001",
    type: ItemType.FEATURE,
    title: "An item",
    priority: Priority.P2,
    rank: "a0",
    columnKey: "ready",
    version: 1n,
    ...over,
  });
}

export function snapshot(cards: readonly ItemCard[] = [], seq = 0n): BoardSnapshot {
  return create(BoardSnapshotSchema, {
    settings: create(BoardSettingsSchema, { projectId: PROJECT, key: "H", staleAfterHours: 24 }),
    columns: defaultColumns(),
    cards: [...cards],
    seq,
  });
}

export function moved(seq: bigint, next: ItemCard, from: string): BoardEvent {
  return create(BoardEventSchema, {
    projectId: PROJECT,
    seq,
    kind: BoardEventKind.ITEM_MOVED,
    itemId: next.id,
    card: next,
    fromColumn: from,
  });
}

export function boardEvent(over: MessageInitShape<typeof BoardEventSchema>): BoardEvent {
  return create(BoardEventSchema, { projectId: PROJECT, ...over });
}

/** A busy board for stories and screenshots. */
export function sampleCards(): ItemCard[] {
  return [
    card({
      id: "H-031",
      title: "Triage the crash report from the iMac",
      type: ItemType.BUG,
      columnKey: "inbox",
      rank: "a0",
    }),
    card({
      id: "H-032",
      title: "Spike: sync over Tailscale",
      type: ItemType.SPIKE,
      columnKey: "inbox",
      rank: "a1",
    }),
    card({
      id: "H-024",
      title: "Releases tab with approval flow",
      columnKey: "ready",
      rank: "a0",
      platforms: [Platform.DESKTOP],
      acTotal: 4,
    }),
    card({
      id: "H-025",
      title: "Rename bot state Ready to Idle",
      type: ItemType.CHORE,
      columnKey: "ready",
      rank: "a1",
      platforms: [Platform.DESKTOP, Platform.IOS],
    }),
    card({
      id: "H-017",
      title: "Backlog and meetings data model in the DB",
      columnKey: "doing",
      priority: Priority.P1,
      assignee: "b1",
      platforms: [Platform.DESKTOP, Platform.IOS, Platform.DAEMON],
      blocked: true,
      stale: true,
      acChecked: 2,
      acTotal: 4,
    }),
    card({
      id: "H-019",
      title: "Board over the WebSocket",
      columnKey: "doing",
      assignee: "b2",
      platforms: [Platform.DAEMON],
      acChecked: 1,
      acTotal: 3,
    }),
    card({
      id: "H-020",
      title: "Dashboard widgets",
      columnKey: "doing",
      assignee: "b2",
      priority: Priority.P0,
      labels: ["wip-override"],
      platforms: [Platform.DESKTOP],
    }),
    card({
      id: "H-012",
      title: "Pairing token errors read as plain words",
      columnKey: "review",
      type: ItemType.BUG,
      assignee: "b1",
      platforms: [Platform.IOS],
      acChecked: 3,
      acTotal: 3,
    }),
    card({
      id: "H-010",
      title: "iOS first run",
      columnKey: "verify",
      assignee: "b1",
      platforms: [Platform.IOS],
      acChecked: 2,
      acTotal: 2,
    }),
    card({
      id: "H-008",
      title: "Permission profiles",
      columnKey: "verify",
      rank: "a1",
      assignee: "b2",
      platforms: [Platform.DAEMON],
    }),
    card({
      id: "H-009",
      title: "Notarised macOS build",
      columnKey: "verify",
      rank: "a2",
      type: ItemType.CHORE,
      platforms: [Platform.INFRA],
    }),
    card({
      id: "H-003",
      title: "Home screen in light mode",
      columnKey: "done",
      platforms: [Platform.DESKTOP],
    }),
  ];
}

export function column(key: string): BoardColumn {
  const found = defaultColumns().find((c) => c.key === key);
  if (found === undefined) {
    throw new Error(`no default column '${key}'`);
  }
  return found;
}

export function unmet(code: string, text: string, fix?: string): Unmet {
  return create(UnmetSchema, { code, text, fix });
}
