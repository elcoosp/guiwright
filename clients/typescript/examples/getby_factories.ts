/**
 * The TS `getBy*` locator factories' consumer demo (the m95 record's
 * named TS pass, its factory half — the one-milestone-per-surface
 * pattern): the five Playwright-shaped factories mirroring the Rust
 * kit's `get_by_*` sugars (`crates/guiwright-kit/src/app.rs`), the
 * string/`TextMatchV1`/`RegExp` probes composing the wire's existing
 * rows client-side — `resolve()` (the strict auto-waited aim), the
 * M47 aim-and-act split, and the never-waiting set read. No wire
 * change: the rows the kit and the REPL already speak, now factory-
 * composed on the TS client.
 *
 * ```text
 * npm run example:getby
 * # spawns the Rust demo app on the default state fixture, then prints
 * # the named/exact/unnamed role reads, the label/text/value probes
 * # (string and RegExp forms), and the honest empty read
 * ```
 *
 * The Rust-side mirrors of these conversations are the kit's
 * `regex_text.rs` example and the REPL's `/pattern/` literal.
 */

import { spawn } from "node:child_process";
import { dirname, resolve as resolvePath } from "node:path";
import { fileURLToPath } from "node:url";
import assert from "node:assert/strict";

import { GwpClient } from "../src/client.ts";
import { getByRole, getByText, getByLabel, getByValue, getByTestId } from "../src/getby.ts";

const here = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolvePath(here, "..", "..", "..");

/** Spawn the Rust demo app on the default state fixture; resolve when
 * it prints `GWP_TCP_ADDR=`. */
async function startDemoApp(): Promise<{ addr: string; stop: () => void }> {
  const child = spawn(
    "cargo",
    [
      "run",
      "-p",
      "guiwright-driver",
      "--example",
      "gwp_demo_app",
      "--",
      "--port",
      "0",
    ],
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
      const match = buffer.match(/GWP_TCP_ADDR=(\S+)/);
      if (match) {
        clearTimeout(timer);
        resolveAddr(match[1]);
      }
    });
    child.on("exit", (code) => reject(new Error(`demo app exited early: ${code}`)));
  });
  return { addr, stop: () => child.kill() };
}

const demo = await startDemoApp();
try {
  const client = await GwpClient.connectTcp(demo.addr);
  try {
    await client.handshake("ts-getby-demo/0.1");

    // The named role factory (the kit's get_by_role mirror): the fuzzy
    // default — substring, case-insensitive — through the strict
    // auto-waited read, then the aim-and-act split.
    const save = await client.resolve(getByRole("Button", { name: "Save" }));
    const clicked = await client.action(save.node, save.generation, { kind: "click" });
    assert.equal(clicked.detail, "clicked", "the factory row aims and acts");
    console.log("role    → getByRole('Button', { name: 'Save' }) resolved and clicked");

    // The exact option (Playwright parity): case-sensitive
    // whole-string — the Rust `TextMatch::exact` words.
    const exact = await client.resolve(getByRole("Button", { name: "Save", exact: true }));
    assert.equal(exact.node.kind, "accesskit", "the exact form resolves");
    console.log("exact   → { name: 'Save', exact: true } resolved the same button");

    // The unnamed role bucket (the kit's role_locator shape): the
    // whole set, document order — the set read never waits.
    const buttons = await client.resolveAll(getByRole("Button"));
    console.log(`bucket  → getByRole('Button') answered ${buttons.length} rows`);

    // The text/label/value probes: the string rows and the regex rows
    // (the pattern's own inline flags decide case — the m95 family).
    const label = await client.resolve(getByLabel("Welcome, Ada"));
    assert.equal(label.node.kind, "accesskit", "the label row resolves");
    const text = await client.resolve(getByText(/Welcome/));
    assert.equal(text.node.kind, "accesskit", "the text regex row resolves");
    const value = await client.resolve(getByValue("ada@example.dev"));
    assert.equal(value.node.kind, "accesskit", "the value row resolves");
    console.log("probes  → getByLabel(string), getByText(/re/), getByValue(string) resolved");

    // The honest empty read: a probe nothing carries answers zero —
    // the read is total, never an error.
    const ghosts = await client.resolveAll(getByLabel(/Ghost/));
    assert.equal(ghosts.length, 0);
    const noIds = await client.resolveAll(getByTestId("btn-save"));
    assert.equal(noIds.length, 0, "this mock door carries no test ids");
    console.log("empty   → a probe nothing carries answered 0 rows (the honest read)");
  } finally {
    await client.close();
  }
} finally {
  demo.stop();
}
