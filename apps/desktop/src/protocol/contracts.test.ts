import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { CONTRACTS } from "./contracts";
import type { BoardContract } from "./generated/board";

// The repo root, from apps/desktop where vitest runs.
const ROOT = join(process.cwd(), "..", "..");

interface ObjectSchema {
  readonly properties?: Readonly<Record<string, unknown>>;
  readonly required?: readonly string[];
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function readJson(path: string): unknown {
  return JSON.parse(readFileSync(path, "utf8"));
}

/** The definition a root property points at, e.g. `item` → `Item`. */
function definitionFor(schema: Record<string, unknown>, field: string): ObjectSchema {
  const properties = isRecord(schema["properties"]) ? schema["properties"] : {};
  const property = properties[field];
  const ref = isRecord(property) && typeof property["$ref"] === "string" ? property["$ref"] : "";
  const definitions = isRecord(schema["definitions"]) ? schema["definitions"] : {};
  const definition = definitions[ref.replace("#/definitions/", "")];
  if (!isRecord(definition)) {
    throw new Error(`no definition for ${field}`);
  }
  return definition;
}

const ENTITIES: readonly (keyof BoardContract)[] = [
  "card",
  "column",
  "comment",
  "event",
  "item",
  "link",
  "role",
  "settings",
  "template",
  "unmet",
];

describe("board contract", () => {
  const schema = readJson(join(ROOT, "contract", "board.schema.json"));
  if (!isRecord(schema)) {
    throw new Error("schema is not an object");
  }

  it("sends the version the schema declares", () => {
    const declared = isRecord(schema["x-contract"]) ? schema["x-contract"]["version"] : undefined;
    expect(CONTRACTS["board"]).toBe(declared);
  });

  it("has a golden fixture for every entity and nothing else", () => {
    const files = readdirSync(join(ROOT, "crates", "bus", "fixtures", "board"))
      .filter((name) => name.endsWith(".json"))
      .map((name) => name.replace(/\.json$/, ""));
    expect(new Set(files)).toEqual(new Set([...ENTITIES, "enums"]));
  });

  it("lists every enum variant the schema allows", () => {
    const fixture = readJson(join(ROOT, "crates", "bus", "fixtures", "board", "enums.json"));
    const definitions = isRecord(schema["definitions"]) ? schema["definitions"] : {};
    const fromSchema = Object.fromEntries(
      Object.entries(definitions).flatMap(([name, definition]) =>
        isRecord(definition) && Array.isArray(definition["enum"])
          ? [[name, definition["enum"]]]
          : [],
      ),
    );
    expect(fixture).toEqual(fromSchema);
  });

  it.each(ENTITIES)("decodes the %s fixture with the generated shape", (entity) => {
    const fixture = readJson(join(ROOT, "crates", "bus", "fixtures", "board", `${entity}.json`));
    if (!isRecord(fixture)) {
      throw new Error(`${entity}.json is not an object`);
    }
    const definition = definitionFor(schema, entity);
    const known = Object.keys(definition.properties ?? {});
    expect(Object.keys(fixture).filter((key) => !known.includes(key))).toEqual([]);
    expect((definition.required ?? []).filter((key) => !(key in fixture))).toEqual([]);
  });
});
