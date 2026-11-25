/**
 * The TS filter() helper's consumer demo (m92's typed `hasText` /
 * `hasNot` / `has` / `nth` opts over the m34/m91 filter rows — the one
 * TS surface without its own runnable example until now): the typed
 * opts compose `Selector` rows byte-identically for every typed method
 * — here `resolve()` (the strict auto-waited aim) and `describeAll()`
 * (the set's facts), no locator object, no wire change.
 *
 * ```text
 * npm run example:filter
 * # spawns the Rust demo app on the default state fixture, then prints
 * # the has-narrowed group row, the Save button's hasText+nth aim and
 * # its ActionDone pair, and the hasNot set read with Cancel dropped
 * ```
 *
 * The Rust-side mirrors of these conversations are the kit's
 * `has_filter.rs` example and the REPL's filter grammar words.
 */

import { spawn } from "node:child_process";
import { dirname, resolve as resolvePath } from "node:path";
import { fileURLToPath } from "node:url";
import assert from "node:assert/strict";

import { GwpClient } from "../src/client.ts";
import { filter } from "../src/filter.ts";

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
    await client.handshake("ts-filter-demo/0.1");

    // The selector-valued positive filter (m91's appended
    // `SelectorV1::Has` row through the helper's `has` opt): the group
    // whose subtree holds a button — the ancestor row, strict and
    // auto-waited.
    const group = await client.resolve(
      filter(
        { kind: "role", role: "Group", name: null },
        { has: { kind: "role", role: "Button", name: null } },
      ),
    );
    assert.equal(group.node.kind, "accesskit", "the has-narrowed group resolves");
    console.log("has     → the group resolved (subtree holds a button)");

    // The string form + the document-order pick in one composition
    // (m34's filter row): the Save button narrowed by the fuzzy
    // default, first in document order — then acted on through the
    // M47 aim-and-act split.
    const save = await client.resolve(
      filter({ kind: "role", role: "Button", name: null }, {
        hasText: "Save",
        nth: "first",
      }),
    );
    const clicked = await client.action(save.node, save.generation, { kind: "click" });
    assert.equal(clicked.detail, "clicked", "the narrowed resolve acts");
    console.log(`hasText+nth → resolved Save and clicked it (frame ${clicked.frame_seq})`);

    // The inverted selector form (m34's `has_not` row): every button
    // whose subtree does NOT speak "Cancel" — the Cancel row drops,
    // the rest answer as describe facts.
    const rows = await client.describeAll(
      filter({ kind: "role", role: "Button", name: null }, {
        hasNot: { kind: "text", match: { text: "Cancel", exact: false, ignore_case: true } },
      }),
    );
    const names = rows.map((r) => r.name);
    assert.ok(!names.includes("Cancel"), "the Cancel row dropped");
    assert.ok(names.includes("Save"), "the Save row stayed");
    console.log(`hasNot   → ${names.length} button rows without Cancel: ${JSON.stringify(names)}`);
  } finally {
    await client.close();
  }
} finally {
  demo.stop();
}
