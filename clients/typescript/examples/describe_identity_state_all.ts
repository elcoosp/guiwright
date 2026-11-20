/**
 * The set-width identity-state describe read's consumer demo (m71 —
 * the m70 Rust seam's `describe_all_identity_state` mirrored on the
 * TypeScript client, the m45/m60/m63/m65/m67/m69
 * one-milestone-per-surface pattern): the m68/m69 records' named
 * standing gap at set width — "no state facts at *set* width: the
 * `DescribeAllIdentity` rows stay identity-row shaped; a state-fact set
 * widening lands when a consumer asks" — closed by the appended
 * `describe_all_identity_state` domain. The m64 `describeAll()` walk
 * payload rides in and the m68 identity-state row rides out per match,
 * so one round trip answers "what does every node in this set say
 * about itself and its state" where the handle-width read answers one
 * handle at a time.
 *
 * ```text
 * npm run example:identity-state-all
 * # spawns the Rust demo app, then prints the checkbox walk answering
 * # the tri-state per row in document order, the disabled button's
 * # enabled: false, and the empty set's honest empty list
 * ```
 *
 * The Rust-side mirror of this conversation is `cargo run -p
 * guiwright-driver --example wire_client_identity_state_all`.
 */

import { spawn } from "node:child_process";
import { dirname, resolve as resolvePath } from "node:path";
import { fileURLToPath } from "node:url";
import assert from "node:assert/strict";

import { GwpClient } from "../src/client.ts";

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
  await client.handshake("ts-identity-state-all-demo/0.1");

  // The whole checkbox match set rides one round trip, document order —
  // the m68 row per match (the identity four, the description fact plus
  // the state three). The demo's state fixture carries the whole M20
  // tri-state.
  const rows = await client.describeAllIdentityState({
    kind: "role",
    role: "CheckBox",
    name: null,
  });
  console.log(`describe_all_identity_state(CheckBox) — ${rows.length} rows, document order:`);
  for (const row of rows) {
    console.log(
      `  ${row.role} ${JSON.stringify(row.name)} focused ${row.focused} enabled ${row.enabled} checked ${row.checked}`,
    );
  }
  assert.deepEqual(
    rows.map((r) => r.checked),
    ["checked", "unchecked", "mixed"],
    "the tri-state rides per row, never squeezed into a bool",
  );

  // A narrowed walk answers one disabled row — the total read, never an
  // error (the m38 rule at set width).
  const disabledRows = await client.describeAllIdentityState({
    kind: "role",
    role: "Button",
    name: { text: "Confirm", exact: true, ignore_case: true },
  });
  const disabledRow = disabledRows[0];
  assert.ok(disabledRow, "the disabled button rides the walk");
  console.log(
    `describe_all_identity_state(Confirm) — enabled ${disabledRow.enabled} checked ${disabledRow.checked}`,
  );

  // An empty set answers with an empty list — the set read is total,
  // never an error (the m30 rule).
  const empty = await client.describeAllIdentityState({
    kind: "role",
    role: "CheckBox",
    name: { text: "No such toggle", exact: true, ignore_case: true },
  });
  console.log(
    `describe_all_identity_state(no match) — ${empty.length} rows (the honest empty list)`,
  );

  await client.close();
  console.log("✓ bye — the state facts at set width green (TypeScript → Rust over TCP)");
} finally {
  demo.stop();
}
