import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { equals, fromBinary, fromJson, toBinary } from "@bufbuild/protobuf";
import type { DescMessage, JsonValue } from "@bufbuild/protobuf";
import { describe, expect, it } from "vitest";
import { CONTRACTS } from "./contracts";
import {
  BoardColumnSchema,
  BoardSettingsSchema,
  file_hermes_board_v1_board,
  ItemCardSchema,
  ItemCommentSchema,
  ItemEventSchema,
  ItemLinkSchema,
  ItemSchema,
  ProjectRoleSchema,
  TemplateSchema,
  UnmetSchema,
} from "./gen/hermes/board/v1/board_pb";
import {
  BoardPushSchema,
  BoardRequestSchema,
  BoardResponseSchema,
  file_hermes_board_v1_requests,
  MoveResultSchema,
} from "./gen/hermes/board/v1/requests_pb";
import {
  AttentionRowsSchema,
  HomeRequestSchema,
  HomeResponseSchema,
  ProjectAttentionSchema,
  ProjectsOverviewSchema,
} from "./gen/hermes/home/v1/home_pb";
import {
  DiskReportSchema,
  PrCommentsSchema,
  PrDiffSchema,
  PrListSchema,
  PrPushSchema,
  PrRequestSchema,
  PrResponseSchema,
  PullRequestSchema,
  ReleaseFromMainSchema,
  ReviewSettingsSchema,
} from "./gen/hermes/pr/v1/pr_pb";
import { EnvelopeSchema } from "./gen/hermes/wire/v1/envelope_pb";
import type { Envelope } from "./gen/hermes/wire/v1/envelope_pb";

// The repo root, from apps/desktop where vitest runs.
const FIXTURES = join(process.cwd(), "..", "..", "crates", "bus", "fixtures", "board");
const MESSAGES = join(FIXTURES, "messages");

const ENTITIES: Readonly<Record<string, DescMessage>> = {
  card: ItemCardSchema,
  column: BoardColumnSchema,
  comment: ItemCommentSchema,
  event: ItemEventSchema,
  item: ItemSchema,
  link: ItemLinkSchema,
  role: ProjectRoleSchema,
  settings: BoardSettingsSchema,
  template: TemplateSchema,
  unmet: UnmetSchema,
};

function isJsonObject(value: unknown): value is { readonly [key: string]: JsonValue } {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function readJson(name: string, dir = FIXTURES): { readonly [key: string]: JsonValue } {
  const parsed: unknown = JSON.parse(readFileSync(join(dir, `${name}.json`), "utf8"));
  if (!isJsonObject(parsed)) {
    throw new Error(`${name}.json is not an object`);
  }
  return parsed;
}

describe("board contract", () => {
  it("speaks board contract version 1", () => {
    expect(CONTRACTS["board"]).toBe(1);
  });

  it("has a golden fixture for every entity and the enum list, nothing else", () => {
    const files = readdirSync(FIXTURES)
      .filter((name) => name.endsWith(".json"))
      .map((name) => name.replace(/\.json$/, ""));
    expect(new Set(files)).toEqual(new Set([...Object.keys(ENTITIES), "enums"]));
  });

  it.each(Object.keys(ENTITIES))(
    "decodes the %s fixture with the generated parser and round-trips it",
    (name) => {
      const schema = ENTITIES[name];
      if (schema === undefined) {
        throw new Error(`no schema for ${name}`);
      }
      const message = fromJson(schema, readJson(name));
      expect(equals(schema, fromBinary(schema, toBinary(schema, message)), message)).toBe(true);
    },
  );

  it("lists every value of every enum", () => {
    const fromProto = Object.fromEntries(
      [...file_hermes_board_v1_board.enums, ...file_hermes_board_v1_requests.enums].map((e) => [
        e.name,
        e.values.filter((v) => v.number !== 0).map((v) => v.name),
      ]),
    );
    expect(readJson("enums")).toEqual(fromProto);
  });
});

/** The proto name of the oneof field set under `localName`. */
function protoName(schema: DescMessage, localName: string | undefined): string {
  const field = schema.fields.find((f) => f.localName === localName);
  if (field === undefined) {
    throw new Error(`${schema.typeName} has no field ${String(localName)}`);
  }
  return field.name;
}

/** `request.<arm>`, `response.<arm>` (`response.moved.<outcome>`), `push.<arm>` or `error`. */
function armOf(envelope: Envelope): string {
  const body = envelope.body;
  switch (body.case) {
    case "boardRequest":
      return `request.${protoName(BoardRequestSchema, body.value.request.case)}`;
    case "boardResponse": {
      const response = body.value.response;
      const arm = protoName(BoardResponseSchema, response.case);
      return response.case === "moved"
        ? `response.${arm}.${protoName(MoveResultSchema, response.value.outcome.case)}`
        : `response.${arm}`;
    }
    case "boardPush":
      return `push.${protoName(BoardPushSchema, body.value.push.case)}`;
    case "homeRequest":
      return `request.${protoName(HomeRequestSchema, body.value.request.case)}`;
    case "homeResponse":
      return `response.${protoName(HomeResponseSchema, body.value.response.case)}`;
    case "prRequest":
      return `request.${protoName(PrRequestSchema, body.value.request.case)}`;
    case "prResponse":
      return `response.${protoName(PrResponseSchema, body.value.response.case)}`;
    case "prPush":
      return `push.${protoName(PrPushSchema, body.value.push.case)}`;
    case "error":
      return "error";
    case undefined:
      throw new Error("an envelope without a body");
  }
}

function names(schema: DescMessage): string[] {
  return schema.fields.map((f) => f.name);
}

/** Every arm a fixture must exist for, read off the generated descriptors. */
function everyArm(): Set<string> {
  return new Set([
    ...names(BoardRequestSchema).map((n) => `request.${n}`),
    ...names(BoardResponseSchema).flatMap((n) =>
      n === "moved" ? names(MoveResultSchema).map((o) => `response.moved.${o}`) : [`response.${n}`],
    ),
    ...names(BoardPushSchema).map((n) => `push.${n}`),
    ...names(HomeRequestSchema).map((n) => `request.${n}`),
    ...names(HomeResponseSchema).map((n) => `response.${n}`),
    ...names(PrRequestSchema).map((n) => `request.${n}`),
    ...names(PrResponseSchema).map((n) => `response.${n}`),
    ...names(PrPushSchema).map((n) => `push.${n}`),
    "error",
  ]);
}

describe("board wire messages", () => {
  const files = readdirSync(MESSAGES)
    .filter((name) => name.endsWith(".json"))
    .map((name) => name.replace(/\.json$/, ""));

  it.each(files)("decodes the %s envelope and round-trips it", (name) => {
    const envelope = fromJson(EnvelopeSchema, readJson(name, MESSAGES));
    expect(
      equals(
        EnvelopeSchema,
        fromBinary(EnvelopeSchema, toBinary(EnvelopeSchema, envelope)),
        envelope,
      ),
    ).toBe(true);
    const arm = armOf(envelope);
    expect(name === arm || name.startsWith(`${arm}.`)).toBe(true);
  });

  it("has a fixture for every request, response and push arm", () => {
    const covered = new Set(
      files.map((name) => armOf(fromJson(EnvelopeSchema, readJson(name, MESSAGES)))),
    );
    expect(covered).toEqual(everyArm());
  });
});

// The projects home's golden fixtures (H-128), shared with the daemon and iOS.
const HOME = join(process.cwd(), "..", "..", "crates", "bus", "fixtures", "home");
const HOME_MESSAGES: Readonly<Record<string, DescMessage>> = {
  overview: ProjectsOverviewSchema,
  attention_rows: AttentionRowsSchema,
  project_attention: ProjectAttentionSchema,
};

describe("home contract", () => {
  it.each(Object.keys(HOME_MESSAGES))("decodes the %s fixture and round-trips it", (name) => {
    const schema = HOME_MESSAGES[name];
    if (schema === undefined) {
      throw new Error(`no schema for ${name}`);
    }
    const message = fromJson(schema, readJson(name, HOME));
    expect(equals(schema, fromBinary(schema, toBinary(schema, message)), message)).toBe(true);
  });
});

// Pull requests and checks (H-265): the golden fixtures the PR tab is built
// on, shared with the daemon and iOS.
const PR = join(process.cwd(), "..", "..", "crates", "bus", "fixtures", "pr");
const PRS = ["pr_open", "pr_behind", "pr_merging", "pr_merged", "pr_closed"];
const PR_MESSAGES: Readonly<Record<string, DescMessage>> = {
  ...Object.fromEntries(PRS.map((name) => [name, PullRequestSchema])),
  pr_list: PrListSchema,
  comments: PrCommentsSchema,
  diff: PrDiffSchema,
  review_settings: ReviewSettingsSchema,
  release_from_main: ReleaseFromMainSchema,
  disk_report: DiskReportSchema,
};

describe("pr contract", () => {
  it("has a schema for every fixture", () => {
    const files = readdirSync(PR)
      .filter((name) => name.endsWith(".json"))
      .map((name) => name.replace(/\.json$/, ""));
    expect(new Set(files)).toEqual(new Set(Object.keys(PR_MESSAGES)));
  });

  it.each(Object.keys(PR_MESSAGES))("decodes the %s fixture and round-trips it", (name) => {
    const schema = PR_MESSAGES[name];
    if (schema === undefined) {
      throw new Error(`no schema for ${name}`);
    }
    const message = fromJson(schema, readJson(name, PR));
    expect(equals(schema, fromBinary(schema, toBinary(schema, message)), message)).toBe(true);
  });
});
