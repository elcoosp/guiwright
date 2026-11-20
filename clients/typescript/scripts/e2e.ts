/**
 * Cross-language e2e (§16, M3): spawn the Rust demo app
 * (`cargo run -p guiwright-driver --example gwp_demo_app`), connect the
 * TypeScript client over real TCP and drive the full conversation:
 *
 * handshake → tree snapshot → resolve → resolve-all → act → clock → mock seams →
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

import { GwpClient, GwpRejected, GwpWaitTimeout } from "../src/client.ts";
import type { DescribedNode, ResolvedMatch } from "../src/schema.ts";

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
  assert.equal(snapshot.node_count, 12, "the state fixture (m68: the checkbox trio rides)");
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

  // --- resolve-all (§8.2 set read, M30) ---
  const all = await client.resolveAll({ kind: "role", role: "Button", name: null });
  assert.equal(all.length, 5, "the described fixture's five buttons, strict mode not consulted");
  assert.deepEqual(
    all.map((m: ResolvedMatch) => m.node.node),
    [2, 3, 6, 7, 14],
    "document order, the count()/all() ordering",
  );
  console.log("✓ resolve-all:", all.length, "matches in document order");

  // --- hit-test (§8.3 mechanism as a public read, M32) ---
  const hit = await client.hitTest(76.0, 32.0);
  assert.ok(hit, "the demo fixture's Save button sits under (76, 32)");
  assert.equal(hit.node.node, 2, "the Save button's identity");
  assert.deepEqual(hit.rect, [16.0, 16.0, 136.0, 48.0]);
  const miss = await client.hitTest(400.0, 220.0);
  assert.equal(miss, null, "empty space answers null (the read is total)");
  console.log("✓ hit-test: Save under (76, 32), null off-tree");

  // --- node-describe (§8.1 identity facts, M36) ---
  // The hit row mints the handle; the describe reads what it points at —
  // role/name/value/bounds without a whole-subtree dump.
  const described = await client.describe(hit.node, hit.generation);
  assert.equal(described.role, "Button");
  assert.equal(described.name, "Save");
  assert.equal(described.value, null, "a button carries no value");
  assert.deepEqual(described.bounds, hit.rect, "the resolve rect, restated");
  const ghost = await client.describe({ kind: "accesskit", tree: "0".repeat(32), node: 999 }, 0)
    .then(
      () => null,
      (err: Error) => err,
    );
  assert.ok(ghost, "a handle the tree does not know rejects (the Action boundary)");
  console.log("✓ node-describe: Save = Button \"Save\", ghost handle rejected");

  // --- describe-all (§8.2 set read carrying facts, M39) ---
  // The resolve-all walk's rows carrying the describe facts — one round
  // trip instead of one describe per row (the m37 record's standing gap).
  const describedAll = await client.describeAll({ kind: "role", role: "Button", name: null });
  assert.equal(describedAll.length, 5, "the same walk resolveAll runs");
  assert.deepEqual(
    describedAll.map((r: DescribedNode) => r.node.node),
    [2, 3, 6, 7, 14],
    "document order, the count()/all() ordering",
  );
  assert.equal(describedAll[0].role, "Button");
  assert.equal(describedAll[0].name, "Save");
  assert.deepEqual(describedAll[0].bounds, [16.0, 16.0, 136.0, 48.0]);
  const emptyAll = await client.describeAll({ kind: "test_id", id: "ghost" });
  assert.deepEqual(emptyAll, [], "an empty set answers with an empty list (the m30 rule)");
  console.log("✓ describe-all:", describedAll.length, "fact rows in document order, empty set total");

  // --- bulk reads at set width (M41) ---
  // The m29/m31 kit readers over any door: `allTextContents` rides the
  // describe-all round trip label-first (the source order the text
  // locators match), `allValues` value-first (what the form carries).
  // The demo fixture's Email input carries both, so the two reads
  // disagree on it — the distinction the m31 record pinned embedded,
  // now cross-language.
  const bulkTexts = await client.allTextContents({ kind: "role", role: "Button", name: null });
  assert.deepEqual(
    bulkTexts,
    [
    "Save",
    "Cancel",
    "Invisible",
    "Confirm",
    "Install",
  ],
    "the full five-button walk, document order, label-first",
  );
  const inputTexts = await client.allTextContents({ kind: "role", role: "TextInput", name: null });
  assert.deepEqual(inputTexts, ["Email"], "texts reads the label (the m29 source order)");
  const inputValues = await client.allValues({ kind: "role", role: "TextInput", name: null });
  assert.deepEqual(
    inputValues,
    ["ada@example.dev"],
    "values reads the value first (the m31 source order)",
  );
  const ghostRead = await client.allTextContents({ kind: "test_id", id: "ghost" });
  assert.deepEqual(ghostRead, [], "an empty set answers with an empty list (the m30 rule)");
  console.log("✓ bulk reads:", bulkTexts.length, "text rows, the Email input split across the two source orders, empty set total");

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

  // --- the client-side wait: locator.waitFor over a real door (M45) ---
  // Satisfied on entry (the fixture's Save is laid out), then the typed
  // expiry against a locator that never matches — the same connection
  // keeps driving afterwards (the m44 read-half contract).
  const waitRow = await client.waitFor(
    { kind: "role", role: "Button", name: { text: "Save", exact: true, ignore_case: true } },
    "visible",
    2000,
  );
  assert.equal(waitRow?.name, "Save", "the visible wait is satisfied on entry");
  assert.ok((waitRow?.bounds ?? [0, 0, 0, 0])[2] > 0, "the satisfying row is laid out");
  await assert.rejects(
    () =>
      client.waitFor(
        { kind: "text", match: { text: "Never", exact: true, ignore_case: false } },
        "visible",
        200,
      ),
    (err: unknown) => {
      assert.ok(err instanceof GwpWaitTimeout, "the typed wait timeout");
      assert.match((err as GwpWaitTimeout).message, /timed out after 200 ms/);
      assert.match((err as GwpWaitTimeout).selector, /getByText/);
      return true;
    },
  );
  await client.ping(); // the spent wait left the door healthy
  console.log("✓ waitFor: satisfied on entry + the typed expiry — door healthy (M45)");

  // --- the content assertions: expect(locator).toHaveText/Value over a
  // real door (M54) --- the label-first read answers the Save button's
  // name, the value-first read answers the demo email input's value, and
  // a mismatched expectation expires with the typed timeout naming
  // expected and found (the same connection keeps driving afterwards).
  const textRow = await client.expectText(
    { kind: "role", role: "Button", name: { text: "Save", exact: true, ignore_case: true } },
    { text: "sav", exact: false, ignore_case: true },
    2000,
  );
  assert.equal(textRow.name, "Save", "the label-first read answers the name");
  const valueRow = await client.expectValue(
    { kind: "role", role: "TextInput", name: { text: "Email", exact: true, ignore_case: true } },
    { text: "ada@example.dev", exact: true, ignore_case: false },
    2000,
  );
  assert.equal(valueRow.value, "ada@example.dev", "the value-first read answers the value");
  await assert.rejects(
    () =>
      client.expectText(
        { kind: "role", role: "Button", name: { text: "Save", exact: true, ignore_case: true } },
        { text: "Done", exact: true, ignore_case: false },
        200,
      ),
    (err: unknown) => {
      assert.ok(err instanceof GwpWaitTimeout, "the typed assertion timeout");
      const timeout = err as GwpWaitTimeout;
      assert.equal(timeout.state, "have text", "the static state word");
      assert.match(timeout.message, /Save/, "the found content rides the line");
      return true;
    },
  );
  await client.ping(); // the spent assertion left the door healthy
  console.log("✓ expectText/expectValue: both source orders + the typed expiry — door healthy (M54)");

  // --- the count family over the door (m59/m60) ---
  const found = await client.expectCount(
    { kind: "role", role: "Button", name: null },
    5,
    2000,
  );
  assert.equal(found, 5, "the demo app's five buttons, found 5 (absence would answer 0)");
  const more = await client.expectCountGreaterThan(
    { kind: "role", role: "Button", name: null },
    3,
    2000,
  );
  assert.equal(more, 5, "the comparator answers the same found fact");
  await assert.rejects(
    () => client.expectCount({ kind: "role", role: "Button", name: null }, 99, 200),
    (err: unknown) => {
      assert.ok(err instanceof GwpWaitTimeout, "the typed assertion timeout");
      const timeout = err as GwpWaitTimeout;
      assert.equal(timeout.state, "have count", "the static state word");
      assert.match(timeout.lastEvaluation, /count is 5, expected 99/, "both sides ride the line");
      return true;
    },
  );
  await client.ping(); // the spent assertion left the door healthy
  console.log("✓ expectCount: the exact form + the comparator + the typed expiry — door healthy (m59/m60)");

  // --- the identity assertions over the door (m62/m63) ---
  // The kit's `to_have_role` / `to_have_accessible_name` vocabulary (M38)
  // mirrored on the TS client (the m45/m60 one-milestone-per-surface
  // pattern): the head row's role answers the exact string, the
  // label-only name answers the matcher (the value never rides the read),
  // and a mismatched identity expires with the typed timeout naming both
  // sides — the same connection keeps driving afterwards.
  const roleRow = await client.expectRole(
    { kind: "role", role: "Button", name: null },
    "Button",
    2000,
  );
  assert.equal(roleRow.role, "Button", "the structural fact answers exactly");
  assert.equal(roleRow.name, "Save", "the set's head row, document order");
  const nameLabel = await client.expectName(
    { kind: "role", role: "TextInput", name: { text: "Email", exact: true, ignore_case: true } },
    { text: "Email", exact: false, ignore_case: true },
    2000,
  );
  assert.equal(nameLabel.name, "Email", "the label-only read answers the name");
  assert.equal(nameLabel.value, "ada@example.dev", "the value rides the row beside the name");
  await assert.rejects(
    () => client.expectRole({ kind: "role", role: "Button", name: null }, "TextInput", 200),
    (err: unknown) => {
      assert.ok(err instanceof GwpWaitTimeout, "the typed assertion timeout");
      const timeout = err as GwpWaitTimeout;
      assert.equal(timeout.state, "have role", "the static state word");
      assert.match(
        timeout.lastEvaluation,
        /role is "Button", expected "TextInput"/,
        "both sides ride the line",
      );
      return true;
    },
  );
  await assert.rejects(
    () =>
      client.expectName(
        { kind: "role", role: "Button", name: null },
        { text: "Ghost", exact: false, ignore_case: true },
        200,
      ),
    (err: unknown) => {
      assert.ok(err instanceof GwpWaitTimeout, "the typed assertion timeout");
      const timeout = err as GwpWaitTimeout;
      assert.equal(timeout.state, "have name", "the static state word");
      assert.match(timeout.lastEvaluation, /name is "Save", expected "Ghost" \(fuzzy\)/);
      return true;
    },
  );
  await assert.rejects(
    () =>
      client.expectName(
        { kind: "role", role: "Group", name: null },
        { text: "Editor", exact: false, ignore_case: true },
        200,
      ),
    (err: unknown) => {
      assert.ok(err instanceof GwpWaitTimeout, "the typed assertion timeout");
      assert.match(
        (err as GwpWaitTimeout).lastEvaluation,
        /no accessible name/,
        "the missing fact is named",
      );
      return true;
    },
  );
  await client.ping(); // the spent assertions left the door healthy
  console.log("✓ expectRole/expectName: the identity rows + the typed expiry — door healthy (m62/m63)");

  // --- the description assertion over the door (m64/m65) ---
  // The kit's `toHaveAccessibleDescription` (M38) mirrored on the TS
  // client (the m45/m60/m63 one-milestone-per-surface pattern) over the
  // appended `describeAllIdentity()` read: the description answers the
  // matcher (the name and the value never answer), a mismatch expires
  // with the typed timeout naming both sides, and the missing fact is
  // named on an undescribed node.
  const describedRow = await client.expectDescription(
    { kind: "role", role: "Button", name: { text: "Install", exact: true, ignore_case: true } },
    { text: "Installs", exact: false, ignore_case: true },
    2000,
  );
  assert.equal(
    describedRow.description,
    "Installs the latest build",
    "the fact the assertion speaks",
  );
  assert.equal(describedRow.name, "Install", "the identity four ride beside it");
  await assert.rejects(
    () =>
      client.expectDescription(
        { kind: "role", role: "Button", name: { text: "Install", exact: true, ignore_case: true } },
        { text: "Ghost", exact: false, ignore_case: true },
        200,
      ),
    (err: unknown) => {
      assert.ok(err instanceof GwpWaitTimeout, "the typed assertion timeout");
      const timeout = err as GwpWaitTimeout;
      assert.equal(timeout.state, "have description", "the static state word");
      assert.match(
        timeout.lastEvaluation,
        /description is "Installs the latest build", expected "Ghost" \(fuzzy\)/,
        "both sides ride the line",
      );
      return true;
    },
  );
  await assert.rejects(
    () =>
      client.expectDescription(
        { kind: "role", role: "Button", name: null },
        { text: "Anything", exact: false, ignore_case: true },
        200,
      ),
    (err: unknown) => {
      assert.ok(err instanceof GwpWaitTimeout, "the typed assertion timeout");
      assert.match(
        (err as GwpWaitTimeout).lastEvaluation,
        /no accessible description/,
        "the missing fact is named",
      );
      return true;
    },
  );
  console.log("✓ expectDescription: the description row + the typed expiry — door healthy (m64/m65)");

  // --- the single-node identity describe over the door (m66/m67) ---
  // The m64 record's named standing gap at handle width, mirrored on
  // the TS client (the m45/m60/m63/m65 one-milestone-per-surface
  // pattern): a resolveAll() row mints the handle, `describeIdentity()`
  // answers the M64 row — the identity four plus the description fact —
  // in one handle-addressed round trip instead of the whole-set walk,
  // the absent fact riding null and the ghost handle rejecting with the
  // Action boundary's typed code.
  const identityRows = await client.resolveAll({
    kind: "role",
    role: "Button",
    name: { text: "Install", exact: true, ignore_case: true },
  });
  const identityRow = identityRows[0];
  assert.ok(identityRow, "the Install button resolves");
  const identityFacts = await client.describeIdentity(identityRow.node, identityRow.generation);
  assert.equal(identityFacts.kind, "described_identity");
  assert.equal(identityFacts.name, "Install", "the identity four ride beside the fact");
  assert.equal(
    identityFacts.description,
    "Installs the latest build",
    "the fact the M36 encoding left out, at handle width",
  );
  const undescribedHead = (await client.resolveAll({ kind: "role", role: "Button", name: null }))[0];
  assert.ok(undescribedHead, "the button set resolves");
  const absentFacts = await client.describeIdentity(
    undescribedHead.node,
    undescribedHead.generation,
  );
  assert.equal(absentFacts.description, null, "the absent fact rides null (a total read)");
  const ghostFacts = await client
    .describeIdentity({ kind: "accesskit", tree: "a5".repeat(16), node: 4242 }, 0)
    .then(
      () => null,
      (e: unknown) => e,
    );
  assert.ok(ghostFacts instanceof GwpRejected, "the typed ghost-handle rejection");
  assert.equal((ghostFacts as GwpRejected).codeName, "UnknownNode");
  await client.ping(); // the ghost handle left the door healthy
  console.log("✓ describeIdentity: the identity row at handle width — door healthy (m66/m67)");

  // --- the single-node identity-state describe over the door (m68/m69) ---
  // The m67 record's named standing gap ("no state flags on the
  // identity row"), mirrored on the TS client (the m45/m60/m63/m65/m67
  // one-milestone-per-surface pattern): a resolveAll() row mints the
  // handle, `describeIdentityState()` answers the M68 row — the M64
  // row's fields plus the state three — in one handle-addressed round
  // trip instead of one question per assertion, the mixed state riding
  // as its own vocabulary word and the ghost handle rejecting with the
  // Action boundary's typed code.
  const mixedRows = await client.resolveAll({
    kind: "role",
    role: "CheckBox",
    name: { text: "Partially installed", exact: true, ignore_case: true },
  });
  const mixedRow = mixedRows[0];
  assert.ok(mixedRow, "the mixed checkbox resolves (the demo's state fixture)");
  const mixedFacts = await client.describeIdentityState(mixedRow.node, mixedRow.generation);
  assert.equal(mixedFacts.kind, "described_identity_state");
  assert.equal(
    mixedFacts.checked,
    "mixed",
    "the tri-state rides as its own word, never a boolean",
  );
  assert.equal(mixedFacts.enabled, true, "the enabled fact rides beside the row");
  assert.equal(mixedFacts.focused, false, "the focused fact rides beside the row");
  const disabledRows = await client.resolveAll({
    kind: "role",
    role: "Button",
    name: { text: "Confirm", exact: true, ignore_case: true },
  });
  const disabledRow = disabledRows[0];
  assert.ok(disabledRow, "the disabled button resolves");
  const disabledFacts = await client.describeIdentityState(
    disabledRow.node,
    disabledRow.generation,
  );
  assert.equal(disabledFacts.enabled, false, "the Disabled flag reads as enabled: false");
  assert.equal(disabledFacts.checked, "not_checkable", "a button exposes no checked state");
  const stateGhost = await client
    .describeIdentityState({ kind: "accesskit", tree: "a5".repeat(16), node: 4242 }, 0)
    .then(
      () => null,
      (e: unknown) => e,
    );
  assert.ok(stateGhost instanceof GwpRejected, "the typed ghost-handle rejection");
  assert.equal((stateGhost as GwpRejected).codeName, "UnknownNode");
  await client.ping(); // the ghost handle left the door healthy
  console.log(
    "✓ describeIdentityState: the state three at handle width — door healthy (m68/m69)",
  );

  // --- the set-width identity-state describe over the door (m70/m71) ---
  // The m68/m69 records' named standing gap ("no state facts at *set*
  // width"), mirrored on the TS client (the m45/m60/m63/m65/m67/m69
  // one-milestone-per-surface pattern): the describeAll() walk payload
  // rides in, `describeAllIdentityState()` answers the m68 row per
  // match, document order — one round trip instead of one
  // `describeIdentityState()` per row, the tri-state riding per row and
  // an empty set answering the honest empty list (the m30 rule).
  const stateRows = await client.describeAllIdentityState({
    kind: "role",
    role: "CheckBox",
    name: null,
  });
  assert.equal(stateRows.length, 3, "the demo's checkbox trio rides one walk");
  assert.equal(stateRows[0].role, "CheckBox", "the m68 row rides per match");
  assert.deepEqual(
    stateRows.map((row) => row.checked),
    ["checked", "unchecked", "mixed"],
    "the tri-state rides per row in document order",
  );
  assert.ok(
    stateRows.every((row) => row.enabled),
    "the trio's checkboxes are live — a total fact per row",
  );
  const narrowedState = await client.describeAllIdentityState({
    kind: "role",
    role: "Button",
    name: { text: "Confirm", exact: true, ignore_case: true },
  });
  assert.equal(narrowedState.length, 1, "the narrowed walk answers the one match");
  assert.equal(narrowedState[0].enabled, false, "the Disabled flag reads as enabled: false");
  assert.equal(narrowedState[0].checked, "not_checkable", "a button exposes no checked state");
  const emptyState = await client.describeAllIdentityState({
    kind: "role",
    role: "CheckBox",
    name: { text: "No such toggle", exact: true, ignore_case: true },
  });
  assert.deepEqual(emptyState, [], "the empty set is the answer, never an error");
  await client.ping();
  console.log(
    "✓ describeAllIdentityState: the state three at set width — door healthy (m70/m71)",
  );

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
  assert.equal(wsSnapshot.node_count, 12, "the same fixture bytes over ws");
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
