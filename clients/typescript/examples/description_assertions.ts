/**
 * The description-assertion demo (m65): `expectDescription()` over the
 * appended `describeAllIdentity()` read against the real Rust demo app
 * — the satisfied row (the description fact beside the identity four),
 * the typed expiry naming both sides, and the missing fact named on an
 * undescribed node.
 *
 * Run from this directory: `npm run example:description` (needs cargo).
 */

import { spawn } from "node:child_process";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { GwpClient, GwpWaitTimeout } from "../src/client.ts";

const repoRoot = join(import.meta.dirname, "..", "..", "..");

const child = spawn(
  "cargo",
  ["run", "-p", "guiwright-driver", "--example", "gwp_demo_app", "--", "--port", "0"],
  { cwd: repoRoot, stdio: ["ignore", "pipe", "inherit"] },
);
const addr = await new Promise<string>((resolve, reject) => {
  let buffer = "";
  const timer = setTimeout(() => reject(new Error("demo app did not print GWP_TCP_ADDR")), 120_000);
  child.stdout.on("data", (chunk) => {
    buffer += chunk.toString();
    const line = buffer.split("\n").find((l) => l.startsWith("GWP_TCP_ADDR="));
    if (line) {
      clearTimeout(timer);
      resolve(line.slice("GWP_TCP_ADDR=".length).trim());
    }
  });
});

const client = await GwpClient.connect(addr, "guiwright-description-demo/0.1");

// Satisfied on entry: the Install button's description answers the
// matcher — the identity four ride beside the fact.
const row = await client.expectDescription(
  { kind: "role", role: "Button", name: { text: "Install", exact: true, ignore_case: true } },
  { text: "Installs", exact: false, ignore_case: true },
  5000,
);
console.log(
  `expectDescription — ${row.description} (the identity four: ${row.role} ${row.name ?? ""})`,
);

// The typed expiry names both sides; the door stays healthy.
try {
  await client.expectDescription(
    { kind: "role", role: "Button", name: { text: "Install", exact: true, ignore_case: true } },
    { text: "Ghost", exact: false, ignore_case: true },
    250,
  );
} catch (err) {
  if (err instanceof GwpWaitTimeout) console.log(`expired — ${err.message}`);
  else throw err;
}

// The missing fact is named on an undescribed node (the Save button).
try {
  await client.expectDescription(
    { kind: "role", role: "Button", name: null },
    { text: "Anything", exact: false, ignore_case: true },
    250,
  );
} catch (err) {
  if (err instanceof GwpWaitTimeout) console.log(`pending fact — ${err.message}`);
  else throw err;
}

await client.bye();
child.kill();
process.exit(0);
