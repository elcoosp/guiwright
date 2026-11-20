/**
 * The state-assertion vocabulary's consumer demo (m75 — the m73 Rust
 * seam's `expect_checked` / `expect_enabled` / `expect_focused` rows
 * mirrored on the TypeScript client, the m45/m60/m63/m65/m67/m69/m71
 * one-milestone-per-surface pattern): the m73 record's named standing
 * gap — "the kit's `to_be_checked` / `to_be_enabled` / `to_be_focused`
 * vocabulary over any door" — closed on the other-language client. The
 * m68 four-way word is demanded exactly (a mixed checkbox is not an
 * unchecked one, a not-checkable role answering the total fact), the
 * enabled boolean answers both ways, and the typed expiry names `be
 * checked` and the word the last evaluation found.
 *
 * ```text
 * npm run example:state-assertions
 * # spawns the Rust demo app, then prints the tri-state asserted on
 * # entry, the enabled pair both ways, the seeded focus answering the
 * # window row, and the typed timeout naming the state word
 * ```
 *
 * The Rust-side mirror of this conversation is `cargo run -p
 * guiwright-driver --example wire_client_assert_state`.
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
  await client.handshake("ts-state-assertions-demo/0.1");

  // The tri-state demanded exactly on entry: the checked, unchecked and
  // mixed words each answer their checkbox — the satisfying m68 row
  // carries the fact (the m73 rule's shape on the other client).
  const checked = await client.expectChecked(
    { kind: "role", role: "CheckBox", name: { text: "Notifications", exact: true, ignore_case: true } },
    "checked",
  );
  console.log(`expectChecked(Notifications, checked) — focused ${checked.focused} enabled ${checked.enabled} checked ${checked.checked}`);

  const unchecked = await client.expectChecked(
    { kind: "role", role: "CheckBox", name: { text: "Dark mode", exact: true, ignore_case: true } },
    "unchecked",
  );
  console.log(`expectChecked("Dark mode", unchecked) — checked ${unchecked.checked}`);

  const mixed = await client.expectChecked(
    { kind: "role", role: "CheckBox", name: { text: "Partially installed", exact: true, ignore_case: true } },
    "mixed",
  );
  console.log(`expectChecked("Partially installed", mixed) — checked ${mixed.checked}`);

  // The four-way word's total fact: a not-checkable role answers
  // `not_checkable` — a fact, never an error.
  const notCheckable = await client.expectChecked(
    { kind: "role", role: "Button", name: { text: "Save", exact: true, ignore_case: true } },
    "not_checkable",
  );
  console.log(`expectChecked(Save, not_checkable) — checked ${notCheckable.checked}`);

  // The enabled pair, both ways: the live checkbox answers true, the
  // disabled `Confirm` answers false (the AccessKit `Disabled` flag).
  const enabled = await client.expectEnabled(
    { kind: "role", role: "CheckBox", name: { text: "Notifications", exact: true, ignore_case: true } },
    true,
  );
  console.log(`expectEnabled(Notifications, enabled) — enabled ${enabled.enabled}`);
  const disabled = await client.expectEnabled(
    { kind: "role", role: "Button", name: { text: "Confirm", exact: true, ignore_case: true } },
    false,
  );
  console.log(`expectEnabled(Confirm, disabled) — enabled ${disabled.enabled} checked ${disabled.checked}`);

  // The focus: the fixture seeds the focus at the window root, so the
  // window row answers — the fact the walk carries.
  const windowRow = await client.expectFocused(
    { kind: "role", role: "Window", name: { text: "Test App", exact: true, ignore_case: true } },
  );
  console.log(`expectFocused("Test App") — focused ${windowRow.focused}`);

  // A spent budget rejects with the typed timeout — `be checked`, the
  // locator, the word the last evaluation found — and the door survives
  // it (the debugging tool outlives its failures).
  try {
    await client.expectChecked(
      { kind: "role", role: "CheckBox", name: { text: "Notifications", exact: true, ignore_case: true } },
      "unchecked",
      200,
    );
    assert.fail("the wrong word must expire");
  } catch (err) {
    assert.ok(err instanceof GwpWaitTimeout, "the typed assertion timeout");
    const timeout = err as GwpWaitTimeout;
    console.log(`expectChecked(Notifications, unchecked) — expired: ${timeout.message}`);
  }
  await client.ping();
  console.log("✓ bye — the state assertions green (TypeScript → Rust over TCP)");

  await client.close();
} finally {
  demo.stop();
}
