/**
 * The appended-action sugars' consumer demo (m89 §5 / m90 §5's named
 * per-surface milestone — "the per-surface sugar is its own milestone
 * when a remote TS suite asks (the m64/m65 pattern)"): the m89
 * `selectOption()` and the m90 `scrollIntoViewIfNeeded()` mirrored on
 * the TypeScript client, one semantics with the kit's
 * `Locator::select_option` / `Locator::scroll_into_view_if_needed` —
 * the M47 aim-and-act rows speaking the appended `SemActionV1`
 * variants (discriminants 10/11, the codec's encode tables
 * byte-pinned since m89/m90), no wire change.
 *
 * ```text
 * npm run example:appended-actions
 * # spawns the Rust demo app on the m89 combobox fixture, then prints
 * # the select round trip (the option by accessible name), the
 * # scroll-into-view round trip, and the ActionDone pairs' frames
 * ```
 *
 * The Rust-side mirrors of these conversations are the kit's
 * `select_option.rs` / `scroll_into_view.rs` examples and the REPL's
 * `select` / `scrollintoview` rows.
 */

import { spawn } from "node:child_process";
import { dirname, resolve as resolvePath } from "node:path";
import { fileURLToPath } from "node:url";
import assert from "node:assert/strict";

import { GwpClient } from "../src/client.ts";

const here = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolvePath(here, "..", "..", "..");

/** Spawn the Rust demo app on the combobox fixture; resolve when it
 * prints `GWP_TCP_ADDR=`. */
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
      "--fixture",
      "combobox",
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
  await client.handshake("ts-appended-actions-demo/0.1");

  // The m89 select vocabulary: the option by accessible name over the
  // appended SelectOption variant — the strict, auto-waited resolve
  // aims the action (the M47 aim-and-act split), the actionability
  // gate rides unchanged, the ActionDone pair answers.
  const selected = await client.selectOption(
    { kind: "role", role: "ComboBox", name: { text: "Country", exact: true, ignore_case: false } },
    "France",
  );
  console.log(
    `select_option — ${JSON.stringify(selected.detail)} at frame ${selected.frame_seq}`,
  );
  assert.equal(selected.detail, 'select_option("France")', "the m89 detail verbatim");

  // The m90 scroll-into-view vocabulary: the a11y-first request over
  // the appended ScrollIntoView variant — no coordinates, no delta,
  // the framework walks its scrollable ancestors.
  const scrolled = await client.scrollIntoViewIfNeeded(
    { kind: "role", role: "Button", name: { text: "Save", exact: true, ignore_case: false } },
  );
  console.log(
    `scroll_into_view_if_needed — ${JSON.stringify(scrolled.detail)} at frame ${scrolled.frame_seq}`,
  );
  assert.equal(scrolled.detail, "scroll_into_view_if_needed", "the m90 detail verbatim");

  await client.close();
  console.log("✓ bye — the appended-action sugars green (TypeScript → Rust over TCP)");
} finally {
  demo.stop();
}
