/**
 * Cross-language e2e (§16, M3): spawn the Rust demo app
 * (`cargo run -p guiwright-driver --example gwp_demo_app`), connect the
 * TypeScript client over real TCP and drive the full conversation:
 *
 * handshake → tree snapshot → resolve → act → clock → mock seams →
 * video start/stop → strict-mode rejection → bye.
 *
 * Run from the repo root or this directory: `npm run e2e` (needs cargo).
 */

import { spawn } from "node:child_process";
import { mkdtempSync, existsSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname, resolve as resolvePath } from "node:path";
import { fileURLToPath } from "node:url";
import assert from "node:assert/strict";

import { GwpClient, GwpRejected } from "../src/client.ts";

const here = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolvePath(here, "..", "..", "..");

/** Spawn the Rust demo app; resolve when it prints GWP_TCP_ADDR=. */
async function startDemoApp(): Promise<{ addr: string; stop: () => void }> {
  const child = spawn(
    "cargo",
    ["run", "-p", "guiwright-driver", "--example", "gwp_demo_app", "--", "--port", "0"],
    { cwd: repoRoot, stdio: ["ignore", "pipe", "inherit"] },
  );
  const addr = await new Promise((resolveAddr, reject) => {
    let buffer = "";
    const timer = setTimeout(
      () => reject(new Error("demo app did not print GWP_TCP_ADDR within 120 s")),
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
  return {
    addr,
    stop: () => {
      child.kill("SIGTERM");
    },
  };
}

async function waitForTcp(addr, timeoutMs = 15_000) {
  const { connect } = await import("node:net");
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    try {
      await new Promise((res, rej) => {
        const s = connect(addr.port, addr.host, () => {
          s.destroy();
          res();
        });
        s.on("error", (e) => {
          s.destroy();
          rej(e);
        });
      });
      return;
    } catch {
      if (Date.now() > deadline) throw new Error(`demo app never accepted on ${addr.host}:${addr.port}`);
      await new Promise((r) => setTimeout(r, 200));
    }
  }
}

const demo = await startDemoApp();
const [host, port] = demo.addr.split(":");
await waitForTcp({ host, port: Number(port) });
const videoDir = mkdtempSync(join(tmpdir(), "gw-ts-e2e-"));

try {
  const client = await GwpClient.connectTcp(demo.addr);

  // --- handshake (§7) ---
  const welcome = await client.handshake("ts-e2e/0.3");
  console.log("✓ handshake:", welcome.session_id, "adapter =", welcome.adapter.name);
  assert.equal(welcome.adapter.name, "mock");
  assert.ok(welcome.adapter.caps.tree_update);

  await client.ping();
  console.log("✓ ping");

  // --- tree snapshot ---
  const snapshot = await client.treeSnapshot();
  assert.equal(snapshot.node_count, 8, "the buttons fixture");
  console.log("✓ tree snapshot:", snapshot.node_count, "nodes");

  // --- resolve (server-side auto-wait, §8.2) ---
  const resolved = await client.resolve({
    kind: "role",
    role: "Button",
    name: { text: "Save", exact: false, ignore_case: true },
  });
  assert.equal(resolved.node.kind, "accesskit");
  assert.equal(resolved.generation, 0);
  console.log("✓ resolve: node", resolved.node.node, "rect", resolved.rect);

  // --- act (§8.4) ---
  const done = await client.action(resolved.node, resolved.generation, { kind: "click" });
  assert.equal(done.detail, "clicked");
  assert.ok(done.frame_seq > 0);
  console.log("✓ action: clicked at frame", done.frame_seq);

  // --- clock (§14 over the wire) ---
  await client.clock({ kind: "install_virtual" });
  // 320 ms = 19 frame-sized steps at 60 Hz (16.67 ms) + one partial step.
  const clock = await client.clock({ kind: "fast_forward", ms: 320 });
  assert.equal(clock.virtual_time_ms, 320);
  assert.equal(clock.steps, 20, "320 ms in frame-sized steps at 60 Hz");
  console.log("✓ clock: fast-forwarded 320 ms in", clock.steps, "steps");

  // --- mock seams (§14 state seams over the wire) ---
  await client.mock({ kind: "install_seam", id: "reset_state", response: "canned" });
  const canned = await client.resetState("seed-42");
  assert.equal(canned, "canned");
  await client.mock({ kind: "clear", id: "reset_state" });
  const real = await client.resetState("seed-42");
  assert.equal(real, "state reset to seed-42");
  const seeded = await client.callSeam("seed_users", "smoke");
  assert.equal(seeded, "3 users seeded");
  console.log("✓ mock: canned override, clear, real hook, direct call");

  // --- video (§10 over the wire) ---
  await client.videoStart({ fps: 25, dir: videoDir });
  await client.treeSnapshot(); // a couple of pumps present frames
  await client.treeSnapshot();
  const video = await client.videoStop();
  assert.ok(video.frames >= 1, "at least one frame recorded");
  assert.ok(existsSync(video.path), "the finalized artifact exists");
  console.log("✓ video:", video.frames, "frames,", video.dropped, "dropped →", video.path);

  // --- strict-mode rejection is a typed error (§8.2) ---
  await assert.rejects(
    () =>
      client.resolve({
        kind: "role",
        role: "Button",
        name: null, // two buttons → strict-mode violation
      }),
    (err) => {
      assert.ok(err instanceof GwpRejected);
      assert.equal(err.codeName, "StrictModeViolation");
      return true;
    },
  );
  console.log("✓ strict-mode rejection is typed");

  await client.close();
  console.log("✓ bye — cross-language e2e green (TypeScript → Rust over TCP)");
} finally {
  demo.stop();
  rmSync(videoDir, { recursive: true, force: true });
}
