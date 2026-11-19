/**
 * The identity-assertion rows' consumer demo (m63 — the m62 Rust seam's
 * `expect_role` / `expect_name` mirrored on the TypeScript client, the
 * m45/m60 one-milestone-per-surface pattern): the kit's
 * `expect(locator).toHaveRole(...)` / `toHaveAccessibleName(...)`
 * vocabulary (M38) over any door, composed client-side on the M45 wake
 * loop. No wire change: the assertions ride the existing `describeAll()`
 * + event domains byte-for-byte.
 *
 * ```text
 * npm run example:identity
 * # spawns the Rust demo app, then prints the role row, the label-only
 * # name row, and the typed timeout naming `have role` and both sides
 * ```
 *
 * The Rust-side mirror of this conversation — including the wake on a
 * scripted rename — is `cargo run -p guiwright-driver --example
 * wire_client_identity` (the demo app has no scripting door, so the
 * wake half rides the same `waitFor()` loop the M45 leg already pins).
 */

import { spawn } from "node:child_process";
import { dirname, resolve as resolvePath } from "node:path";
import { fileURLToPath } from "node:url";
import assert from "node:assert/strict";

import { GwpClient, GwpWaitTimeout } from "../src/client.ts";

const here = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolvePath(here, "..", "..", "..");

/** Spawn the Rust demo app; resolve when it prints `GWP_TCP_ADDR=`. */
async function startDemoApp(): Promise<{ addr: string; stop: () => void }> {
  const child = spawn(
    "cargo",
    ["run", "-p", "guiwright-driver", "--example", "gwp_demo_app", "--", "--port", "0"],
    { cwd: repoRoot, stdio: ["ignore", "pipe", "inherit"] },
  );
  const addr = await new Promise<string>((resolveAddr, reject) => {
    let buffer = "";
    const timer = setTimeout(
      () => reject(new Error("demo app did not print GWP_TCP_ADDR= within 120 s")),
      120_000,
    );
    child.stdout.on("data", (chunk) => {
      buffer += chunk.toString();
      const line = buffer.split("\n").find((l) => l.startsWith("GWP_TCP_ADDR="));
      if (line) {
        clearTimeout(timer);
        resolveAddr(line.trim().slice("GWP_TCP_ADDR=".length));
      }
    });
    child.on("exit", (code) => {
      clearTimeout(timer);
      reject(new Error(`demo app exited early with ${code}`));
    });
  });
  return { addr, stop: () => child.kill("SIGTERM") };
}

const demo = await startDemoApp();
try {
  const client = await GwpClient.connectTcp(demo.addr);
  await client.handshake("ts-identity-demo/0.1");

  // The role row answers on entry: the head row of the four-button set
  // is the Save button, and the role compares exactly (a structural
  // fact — the AccessKit debug name the describe rows print).
  const roleRow = await client.expectRole(
    { kind: "role", role: "Button", name: null },
    "Button",
    5000,
  );
  console.log(`expect_role — ${roleRow.role} ${JSON.stringify(roleRow.name)} (generation ${roleRow.generation})`);

  // The label-only name row answers the matcher: the Email input's name
  // is "Email" even though the row carries the value beside it — the
  // value never answers a name assertion (the m62 narrower read).
  const nameRow = await client.expectName(
    { kind: "role", role: "TextInput", name: { text: "Email", exact: true, ignore_case: true } },
    { text: "Email", exact: false, ignore_case: true },
    5000,
  );
  console.log(
    `expect_name — ${JSON.stringify(nameRow.name)} (value ${JSON.stringify(nameRow.value)} rides beside it)`,
  );

  // A spent budget prints the typed timeout — `have role`, the locator,
  // the role the last evaluation saw (failures name names).
  const err = await client
    .expectRole({ kind: "role", role: "Button", name: null }, "TextInput", 200)
    .then(
      () => null,
      (e: unknown) => e,
    );
  assert.ok(err instanceof GwpWaitTimeout, "the typed assertion timeout");
  console.log(`expect_role mismatch — ${err.message}`);

  await client.close();
  console.log("✓ bye — the identity rows' demo green (TypeScript → Rust over TCP)");
} finally {
  demo.stop();
}
