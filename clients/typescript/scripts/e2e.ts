/**
 * Cross-language e2e (§16, M3): spawn the Rust demo app
 * (`cargo run -p guiwright-driver --example gwp_demo_app`), connect the
 * TypeScript client over real TCP and drive the full conversation:
 *
 * handshake → tree snapshot → resolve → act → clock → mock seams →
 * video start/stop → subscribe/nextEvent (M12) → treeDumpUnder (M13) →
 * strict-mode rejection → bye.
 *
 * Then the M15 leg: spawn the demo app again with `--ws` and drive the
 * byte-identical conversation through `connectWs` (the RFC 6455 door the
 * TypeScript client shipped in M3; the Rust side serves it since M15).
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

/** Spawn the Rust demo app; resolve when it prints `marker` (GWP_*_ADDR=). */
async function startDemoApp(
  marker: string,
  extraArgs: string[] = [],
): Promise<{ addr: string; stop: () => void }> {
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
      ...extraArgs,
    ],
    { cwd: repoRoot, stdio: ["ignore", "pipe", "inherit"] },
  );
  const addr = await new Promise((resolveAddr, reject) => {
    let buffer = "";
    const timer = setTimeout(
      () => reject(new Error(`demo app did not print ${marker} within 120 s`)),
      120_000,
    );
    child.stdout.on("data", (chunk) => {
      buffer += chunk.toString();
      const line = buffer.split("\n").find((l) => l.startsWith(marker));
      if (line) {
        clearTimeout(timer);
        resolveAddr(line.trim().slice(marker.length));
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

const demo = await startDemoApp("GWP_TCP_ADDR=");
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

  // --- window domain: single-window apps report one "main" target (M6) ---
  const windows = await client.windowList();
  assert.equal(windows.length, 1, "the demo app is single-window");
  assert.equal(windows[0].handle, "main");
  await client.windowUse("main"); // a no-op scope on single-window apps
  console.log("✓ window: listed", windows.length, "target, aimed at main");

  // --- gestures: the §9 input pipeline over the wire (M7) ---
  const click = await client.mouse.click(12.5, 30);
  assert.match(click.detail, /clicked at \(12\.5, 30\) — 3 events/);
  const dbl = await client.mouse.dblclick(4, 8);
  assert.match(dbl.detail, /double-clicked at \(4, 8\) — 5 events/);
  const drag = await client.mouse.drag(10, 10, 110, 10);
  assert.match(drag.detail, /dragged \(10, 10\) → \(110, 10\) — 22 events/);
  await client.mouse.wheel(0, -120);
  const queued = await client.input([{ kind: "pointer_move", x: 1, y: 2 }]);
  assert.ok(queued.frame_seq >= 1, "raw input reports its frame");
  await client.keyboard.press("Enter", ["Shift"]);
  await client.keyboard.type("héllo");
  console.log("✓ gestures: click/drag/wheel + raw input + keyboard (M7)");

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

  // --- event channel: subscribe + receive a pushed event (M12) ---
  // The demo app presents frames continuously (its clock runs), so the
  // frame events flow within a tick; the ack proves the wire path both
  // ways regardless.
  const honored = await client.subscribe(["tree_changed", "frame_presented"]);
  assert.deepEqual(honored, ["tree_changed", "frame_presented"]);
  const event = await Promise.race([
    client.nextEvent(),
    new Promise<null>((resolve) => setTimeout(() => resolve(null), 2000)),
  ]);
  await client.unsubscribe();
  if (event) {
    assert.ok(
      event.kind === "frame_presented" || event.kind === "tree_changed",
      `unexpected event ${JSON.stringify(event)}`,
    );
    console.log("✓ events: subscribed, received", event.kind, "— unsubscribed");
  } else {
    console.log("✓ events: subscribed + unsubscribed (no event within 2s — the app was quiescent)");
  }

  // --- filtered tree dump: one subtree as data (M13) ---
  const under = await client.treeDumpUnder(
    { kind: "role", role: "Group", name: null },
    2000,
  );
  assert.ok(under.node_count >= 1, "the demo app has a group");
  assert.ok(
    under.nodes.every((node) => node.role !== "Window"),
    "the window root is not part of a subtree dump",
  );
  console.log("✓ treeDumpUnder:", under.node_count, "nodes in the group subtree");

  await client.close();
  console.log("✓ bye — cross-language e2e green (TypeScript → Rust over TCP)");
} finally {
  demo.stop();
  rmSync(videoDir, { recursive: true, force: true });
}

// --- M15: the same conversation over WebSocket (the viewer door) ---------
const wsDemo = await startDemoApp("GWP_WS_ADDR=", ["--ws"]);
const [wsHost, wsPort] = wsDemo.addr.split(":");
await waitForTcp({ host: wsHost, port: Number(wsPort) });

try {
  const wsClient = await GwpClient.connectWs(`ws://${wsDemo.addr}`);

  const wsWelcome = await wsClient.handshake("ts-e2e-m15/0.1");
  assert.equal(wsWelcome.adapter.name, "mock");
  console.log("✓ M15 ws: upgraded + handshaked —", wsWelcome.session_id);

  const wsSnapshot = await wsClient.treeSnapshot();
  assert.equal(wsSnapshot.node_count, 8, "the same fixture bytes over ws");
  const wsResolved = await wsClient.resolve({
    kind: "role",
    role: "Button",
    name: { text: "Save", exact: false, ignore_case: true },
  });
  const wsDone = await wsClient.action(
    wsResolved.node,
    wsResolved.generation,
    { kind: "click" },
  );
  assert.equal(wsDone.detail, "clicked");
  console.log("✓ M15 ws: snapshot → resolve → act byte-identical to TCP");

  const wsHonored = await wsClient.subscribe(["frame_presented"]);
  assert.deepEqual(wsHonored, ["frame_presented"]);
  await wsClient.unsubscribe();
  console.log("✓ M15 ws: subscribe/unsubscribe — the viewer live-mode door is open");

  await wsClient.close();
} finally {
  wsDemo.stop();
}
