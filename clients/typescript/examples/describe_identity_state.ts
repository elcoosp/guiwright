/**
 * The single-node identity-state describe read's consumer demo (m69 —
 * the m68 Rust seam's `describe_identity_state` mirrored on the
 * TypeScript client, the m45/m60/m63/m65/m67 one-milestone-per-surface
 * pattern): the m67 record's named standing gap at handle width — the
 * M36 `describe()` handle pair rides in and the M68 identity-state row
 * rides out (the M64 identity row's fields plus the state three), so
 * one handle-addressed round trip answers "what does this node say
 * about itself and its state" where the assertion vocabulary answers
 * one question at a time.
 *
 * ```text
 * npm run example:identity-state
 * # spawns the Rust demo app, then prints the state read answering
 * # resolveAll() handles — the checked / unchecked / mixed tri-state
 * # beside the identity four, the disabled button's enabled: false, an
 * # unfocused node's focused: false — and the ghost handle's typed
 * # rejection
 * ```
 *
 * The Rust-side mirror of this conversation is `cargo run -p
 * guiwright-driver --example wire_client_identity_state`.
 */

import { spawn } from "node:child_process";
import { dirname, resolve as resolvePath } from "node:path";
import { fileURLToPath } from "node:url";
import assert from "node:assert/strict";

import { GwpClient, GwpRejected } from "../src/client.ts";

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
  await client.handshake("ts-identity-state-demo/0.1");

  // A resolveAll() row mints the handle (the dump-row rule, M11); the
  // state read answers it — the M64 row's fields plus the state three,
  // one round trip instead of one question per assertion. The demo's
  // state fixture carries the whole M20 tri-state.
  for (const name of ["Notifications", "Dark mode", "Partially installed"]) {
    const rows = await client.resolveAll({
      kind: "role",
      role: "CheckBox",
      name: { text: name, exact: true, ignore_case: true },
    });
    const row = rows[0];
    assert.ok(row, `the ${name} checkbox resolves`);
    const described = await client.describeIdentityState(row.node, row.generation);
    console.log(
      `describe_identity_state — ${described.role} ${JSON.stringify(described.name)} checked ${described.checked} (generation ${described.generation})`,
    );
  }

  // The disabled node answers `enabled: false` and a node that exposes
  // no checked state answers "not_checkable" — the total read, never an
  // error (the m38 rule at state width).
  const disabledRows = await client.resolveAll({
    kind: "role",
    role: "Button",
    name: { text: "Confirm", exact: true, ignore_case: true },
  });
  const disabledRow = disabledRows[0];
  assert.ok(disabledRow, "the disabled button resolves");
  const disabled = await client.describeIdentityState(disabledRow.node, disabledRow.generation);
  console.log(
    `describe_identity_state — ${disabled.role} ${JSON.stringify(disabled.name)} enabled ${disabled.enabled} checked ${disabled.checked}`,
  );

  // An unfocused node answers false (a total fact, never an error —
  // the state facts are always named).
  const saveRows = await client.resolveAll({
    kind: "role",
    role: "Button",
    name: { text: "Save", exact: true, ignore_case: true },
  });
  const saveRow = saveRows[0];
  assert.ok(saveRow, "the Save button resolves");
  const save = await client.describeIdentityState(saveRow.node, saveRow.generation);
  console.log(
    `describe_identity_state — ${save.role} ${JSON.stringify(save.name)} focused ${save.focused}`,
  );

  // A handle the tree does not know rides the typed rejection — the
  // Action boundary's honesty, the same error an action on the ghost
  // row would meet.
  const ghost = { kind: "accesskit" as const, tree: "a5".repeat(16), node: 4242 };
  const err = await client
    .describeIdentityState(ghost, 0)
    .then(
      () => null,
      (e: unknown) => e,
    );
  assert.ok(err instanceof GwpRejected, "the typed rejection");
  assert.equal((err as GwpRejected).codeName, "UnknownNode");
  console.log(`describe_identity_state ghost handle — ${(err as GwpRejected).message}`);

  await client.close();
  console.log("✓ bye — the state facts at handle width green (TypeScript → Rust over TCP)");
} finally {
  demo.stop();
}
