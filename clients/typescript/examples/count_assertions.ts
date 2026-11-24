/**
 * The count-assertion family's consumer demo (the m25 widening's
 * TypeScript half — the m60 mirror pattern: the m25 record's reserved
 * `at_least`/`at_most` forms and the range shape the same honest list
 * names beside them, riding the same `expectCountWith()` composition on
 * the other-language client). The whole family answers over one door —
 * the exact form, the one-sided comparators, the inclusive pair on both
 * bounds, the range within both ends — and the typed expiry names the
 * demanded form (`count >= 99` / `99..=120`) the last evaluation missed.
 *
 * ```text
 * npm run example:count
 * # spawns the Rust demo app, then prints the found count answered by
 * # every comparison, the inclusive bounds holding on the bound itself,
 * # and the typed timeout naming `have count` and both sides
 * ```
 *
 * The Rust-side mirror of this conversation is `cargo run -p
 * guiwright-driver --example wire_client_count`.
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
  await client.handshake("ts-count-demo/0.1");

  const buttons = { kind: "role" as const, role: "Button" as const, name: null };

  // The exact form and the one-sided comparators: the same set answers
  // every comparison with the found count (the m59/m60 rows).
  const found = await client.expectCount(buttons, 5);
  console.log(`expectCount(Button, 5) — found ${found}`);
  const more = await client.expectCountGreaterThan(buttons, 3);
  console.log(`expectCountGreaterThan(Button, 3) — found ${more}`);
  const fewer = await client.expectCountLessThan(buttons, 6);
  console.log(`expectCountLessThan(Button, 6) — found ${fewer}`);

  // The inclusive pair (m25's honest-list widening): the bound itself
  // holds — `>= 4` on 5, `<= 5` on 5 — the same waitable composition.
  // The range shape the same honest list names beside the pair: 5 sits
  // inside 3..=5, the bounds included.
  const least = await client.expectCountAtLeast(buttons, 4);
  console.log(`expectCountAtLeast(Button, 4) — found ${least}`);
  const most = await client.expectCountAtMost(buttons, 5);
  console.log(`expectCountAtMost(Button, 5) — found ${most}`);
  const between = await client.expectCountBetween(buttons, 3, 5);
  console.log(`expectCountBetween(Button, 3, 5) — found ${between}`);

  // A spent budget rejects with the typed timeout — `have count`, the
  // locator, the inclusive form and the count the last evaluation
  // found — and the door survives it.
  try {
    await client.expectCountAtLeast(buttons, 99, 200);
    assert.fail("the unmet bound must expire");
  } catch (err) {
    assert.ok(err instanceof GwpWaitTimeout, "the typed assertion timeout");
    const timeout = err as GwpWaitTimeout;
    console.log(`expectCountAtLeast(Button, 99) — expired: ${timeout.message}`);
  }
  try {
    await client.expectCountBetween(buttons, 99, 120, 200);
    assert.fail("the unmet range must expire");
  } catch (err) {
    assert.ok(err instanceof GwpWaitTimeout, "the typed assertion timeout");
    const timeout = err as GwpWaitTimeout;
    console.log(`expectCountBetween(Button, 99, 120) — expired: ${timeout.message}`);
  }
  await client.ping();
  console.log("✓ bye — the count family green (TypeScript → Rust over TCP)");

  await client.close();
} finally {
  demo.stop();
}
