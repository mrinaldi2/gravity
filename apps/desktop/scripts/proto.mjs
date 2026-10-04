// Lints the wire contract (proto/) and generates its TypeScript with buf and
// protobuf-es into src/protocol/gen (ADR-001 §1). With --check it generates
// into a scratch directory instead and fails when the committed files differ,
// so CI catches a .proto change without the regenerated code.
import { execFileSync } from "node:child_process";
import { mkdtempSync, readdirSync, readFileSync, rmSync, statSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const desktop = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const root = resolve(desktop, "..", "..");
const buf = join(desktop, "node_modules", ".bin", "buf");
const committed = join(desktop, "src", "protocol", "gen");
const check = process.argv.includes("--check");

function files(dir) {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    return statSync(path).isDirectory() ? files(path) : [path];
  });
}

execFileSync(buf, ["lint"], { cwd: root, stdio: "inherit" });

if (!check) {
  execFileSync(buf, ["generate"], { cwd: root, stdio: "inherit" });
  process.stdout.write(`generated ${relative(root, committed)}\n`);
  process.exit(0);
}

const scratch = mkdtempSync(join(tmpdir(), "hermes-proto-"));
try {
  execFileSync(buf, ["generate", "--output", scratch], { cwd: root, stdio: "inherit" });
  const fresh = join(scratch, relative(root, committed));
  const want = files(fresh)
    .map((path) => relative(fresh, path))
    .sort();
  const have = files(committed)
    .map((path) => relative(committed, path))
    .sort();
  const stale = [
    ...want.filter(
      (name) =>
        !have.includes(name) ||
        readFileSync(join(fresh, name), "utf8") !== readFileSync(join(committed, name), "utf8"),
    ),
    ...have.filter((name) => !want.includes(name)),
  ];
  if (stale.length > 0) {
    process.stderr.write(`src/protocol/gen is stale (${stale.join(", ")}): run pnpm proto:gen\n`);
    process.exitCode = 1;
  }
} finally {
  rmSync(scratch, { recursive: true, force: true });
}
