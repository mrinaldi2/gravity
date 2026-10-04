// Generates the TypeScript for the daemon's typed wire contract from
// contract/<surface>.schema.json (written by `cargo run -p bus --features
// schema --bin contract`). With --check it leaves the file alone and fails
// when the committed copy differs, so CI catches a schema change without the
// regenerated types.
import { execFileSync } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { compile } from "json-schema-to-typescript";

const desktop = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const SURFACES = ["board"];
const check = process.argv.includes("--check");

const BANNER = (surface) =>
  `// Generated from contract/${surface}.schema.json by scripts/contract.mjs. Do not edit;\n` +
  "// change the Rust types in crates/bus/src/contract and regenerate.";

const compiled = await Promise.all(
  SURFACES.map(async (surface) => {
    const schema = JSON.parse(
      readFileSync(join(desktop, "..", "..", "contract", `${surface}.schema.json`), "utf8"),
    );
    const code = await compile(schema, schema.title ?? surface, {
      bannerComment: BANNER(surface),
      additionalProperties: false,
      unreachableDefinitions: true,
      format: false,
    });
    return { surface, code };
  }),
);

let stale = false;
for (const { surface, code } of compiled) {
  const target = join(desktop, "src", "protocol", "generated", `${surface}.ts`);
  let previous = null;
  try {
    previous = readFileSync(target, "utf8");
  } catch {
    // first generation
  }
  writeFileSync(target, code);
  // The repo's own formatter, so the file reads like the rest of src/.
  execFileSync(join(desktop, "node_modules", ".bin", "oxfmt"), [target], { cwd: desktop });
  const formatted = readFileSync(target, "utf8");
  if (check && formatted !== previous) {
    stale = true;
    if (previous === null) {
      process.stderr.write(`${target} is missing\n`);
    } else {
      writeFileSync(target, previous);
      process.stderr.write(`${target} is stale: run pnpm contract:ts\n`);
    }
  } else if (!check) {
    process.stdout.write(`wrote ${target}\n`);
  }
}
process.exit(stale ? 1 : 0);
