/**
 * The release build gate for the TypeScript client (§16): the package
 * ships TypeScript source directly (`main`/`types` point at `src/`,
 * `files` publishes the source tree + schema), so "build" means the
 * shipped surface is *provably loadable and complete* — every module
 * under `src/` parses and loads on the shipped Node floor (type
 * stripping is the compile), the schema JSON parses, and every file
 * the `files` field names exists. The release workflow's
 * `npm publish --dry-run` packs behind this gate: a module that fails
 * to load, a schema that stopped parsing, or a `files` entry that
 * drifted away fails the gate before any artifact is cut.
 *
 * Run: `npm run build` (zero dependencies — the gate itself rides the
 * same Node stdlib the client does).
 */
import { readdirSync, existsSync, statSync, readFileSync } from "node:fs";
import { join, dirname, relative } from "node:path";
import { fileURLToPath } from "node:url";

const pkgDir = join(dirname(fileURLToPath(import.meta.url)), "..");
const pkg = JSON.parse(readFileSync(join(pkgDir, "package.json"), "utf8"));

let failures = 0;
const fail = (msg) => {
  console.error(`build: FAIL ${msg}`);
  failures += 1;
};

// 1. The pack surface: every `files` entry exists (a renamed or
// deleted directory silently shrinks the published tarball).
for (const entry of pkg.files ?? []) {
  if (!existsSync(join(pkgDir, entry))) {
    fail(`files entry is missing on disk: ${entry}`);
  }
}

// 2. The entry points resolve: `main` and `types` name real files.
for (const field of ["main", "types"]) {
  const p = pkg[field];
  if (!p || !existsSync(join(pkgDir, p))) {
    fail(`${field} does not resolve: ${p ?? "(unset)"}`);
  }
}

// 3. The published schema parses and stays an object (the wire
// contract the client is generated against, §7).
const schemaPath = join(pkgDir, "gwp-schema.json");
try {
  const schema = JSON.parse(readFileSync(schemaPath, "utf8"));
  if (typeof schema !== "object" || schema === null || Array.isArray(schema)) {
    fail("gwp-schema.json is not a JSON object");
  }
} catch (err) {
  fail(`gwp-schema.json does not parse: ${err.message}`);
}

// 4. Every shipped module loads: the type strip is the compile for a
// source-shipping package — a syntax error, a dangling import, or a
// top-level crash surfaces here instead of inside a user's `require`.
// `src` is walked so a new module is covered without editing the gate.
const srcDir = join(pkgDir, "src");
const modules = readdirSync(srcDir)
  .filter((f) => f.endsWith(".ts"))
  .sort();
for (const mod of modules) {
  const path = join(srcDir, mod);
  if (!statSync(path).isFile()) continue;
  try {
    await import(path);
  } catch (err) {
    fail(`module failed to load: ${relative(pkgDir, path)} — ${err.message}`);
  }
}

if (failures > 0) {
  console.error(`build: ${failures} failure(s)`);
  process.exit(1);
}
console.log(
  `build: ok — ${modules.length} modules load, schema parses, pack surface complete (${pkg.name}@${pkg.version})`,
);
