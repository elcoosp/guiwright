/**
 * The single-node identity describe read's consumer demo (m67 — the
 * m66 Rust seam's `describe_identity` mirrored on the TypeScript
 * client, the m45/m60/m63/m65 one-milestone-per-surface pattern): the
 * m64 record's named standing gap at handle width — the M36
 * `describe()` handle pair rides in and the M64 identity row rides out,
 * so one handle-addressed round trip answers "what does this node say
 * about itself" where `describeAllIdentity()` walks the set.
 *
 * ```text
 * npm run example:describe-identity
 * # spawns the Rust demo app, then prints the identity row answering a
 * # resolveAll() handle — the description fact named beside the
 * # identity four — an undescribed node riding null, and the ghost
 * # handle's typed rejection
 * ```
 *
 * The Rust-side mirror of this conversation is `cargo run -p
 * guiwright-driver --example wire_client_describe_identity`.
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
  await client.handshake("ts-describe-identity-demo/0.1");

  // A resolveAll() row mints the handle (the dump-row rule, M11); the
  // identity read answers it — the M64 row's identity four plus the
  // description fact, one round trip instead of the whole-set walk.
  const rows = await client.resolveAll({
    kind: "role",
    role: "Button",
    name: { text: "Install", exact: true, ignore_case: true },
  });
  const row = rows[0];
  assert.ok(row, "the Install button resolves");
  const described = await client.describeIdentity(row.node, row.generation);
  console.log(
    `describe_identity — ${described.role} ${JSON.stringify(described.name)} (generation ${described.generation})`,
  );
  console.log(
    `  description ${JSON.stringify(described.description)} — the fact the M36 encoding left out`,
  );

  // An undescribed node rides null (a total read, never an error — the
  // m38 rule: the name and the value never answer a description read).
  const head = (await client.resolveAll({ kind: "role", role: "Button", name: null }))[0];
  assert.ok(head, "the button set resolves");
  const undescribed = await client.describeIdentity(head.node, head.generation);
  console.log(
    `describe_identity — ${undescribed.role} ${JSON.stringify(undescribed.name)} description ${JSON.stringify(undescribed.description)}`,
  );
  assert.equal(undescribed.description, null, "the absent fact rides null");

  // A handle the tree does not know rides the typed rejection — the
  // Action boundary's honesty, the same error an action on the ghost
  // row would meet.
  const ghost = { kind: "accesskit" as const, tree: "a5".repeat(16), node: 4242 };
  const err = await client
    .describeIdentity(ghost, 0)
    .then(
      () => null,
      (e: unknown) => e,
    );
  assert.ok(err instanceof GwpRejected, "the typed rejection");
  assert.equal((err as GwpRejected).codeName, "UnknownNode");
  console.log(`describe_identity ghost handle — ${(err as GwpRejected).message}`);

  await client.close();
  console.log("✓ bye — the identity read at handle width green (TypeScript → Rust over TCP)");
} finally {
  demo.stop();
}
