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

// The repo root, from apps/desktop where vitest runs.
const FIXTURES = join(process.cwd(), "..", "..", "crates", "bus", "fixtures", "board");

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

function readJson(name: string): { readonly [key: string]: JsonValue } {
  const parsed: unknown = JSON.parse(readFileSync(join(FIXTURES, `${name}.json`), "utf8"));
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
      file_hermes_board_v1_board.enums.map((e) => [
        e.name,
        e.values.filter((v) => v.number !== 0).map((v) => v.name),
      ]),
    );
    expect(readJson("enums")).toEqual(fromProto);
  });
});
