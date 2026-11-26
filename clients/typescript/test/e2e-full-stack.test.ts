/**
 * The grand cross-language e2e (§16, M3 — m88): the FULL 1.0.0 stack
 * over the language boundary, against the real Rust demo server the CI
 * `clients` job drives (`cargo run -p guiwright-driver --example
 * gwp_demo_app`, the mock-adapter GWP server over real TCP). Three legs,
 * no protocol change — every leg rides wire vocabulary that already
 * shipped:
 *
 *  1. the §13 remote codegen recording (M4): the TypeScript client
 *     starts the recorder on the session, performs a real click and a
 *     real fill through the wire Action path, stops, and asserts the
 *     exact kit code the Rust recorder emitted (the same lines the
 *     Rust-side `codegen_wire` test pins — one cost model, two clients);
 *  2. the double-stop clean rejection (the recorded domain error, §13);
 *  3. the §12 live trace HTML (m87): the Rust `guiwright live-html`
 *     emitter attaches to the SAME session as a second connection while
 *     the TypeScript client acts (one session thread serves every
 *     connection, §5.1), and the emitted scrubber carries the
 *     `gwtrace/v1 (live)` stamp, the adapter header, the test label and
 *     the captured tree.
 *
 * The Rust binaries are built once up front (`cargo build --locked`,
 * the CI lockfile discipline) and spawned directly — no `cargo run`
 * build-lock contention between the demo server and the emitter.
 *
 * Run: `npm test` (this file is part of the enumeration) or
 * `node --test test/e2e-full-stack.test.ts`. Needs cargo on PATH.
 */

import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { existsSync, readFileSync, mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname, resolve as resolvePath } from "node:path";
import { fileURLToPath } from "node:url";

import {
  GwpClient,
  GwpRejected,
  filter,
  getByLabel,
  getByRole,
  getByTestId,
  getByText,
  getByValue,
  roleNameRegex,
  textRegex,
  labelRegex,
} from "../src/client.ts";

const here = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolvePath(here, "..", "..", "..");
const demoBin = join(repoRoot, "target", "debug", "examples", "gwp_demo_app");
const cliBin = join(repoRoot, "target", "debug", "guiwright");

let demo: { addr: string; stop: () => void };
let workDir: string;

/** Build the two Rust binaries the legs drive (the CI --locked gate). */
function buildBinaries(): Promise<void> {
  return new Promise((resolveBuild, reject) => {
    const child = spawn(
      "cargo",
      [
        "build",
        "--locked",
        "-p",
        "guiwright-driver",
        "--example",
        "gwp_demo_app",
        "-p",
        "guiwright-cli",
        "--bin",
        "guiwright",
      ],
      { cwd: repoRoot, stdio: ["ignore", "ignore", "pipe"] },
    );
    let stderr = "";
    child.stderr.on("data", (chunk: Buffer) => {
      stderr += chunk.toString();
    });
    child.on("exit", (code) => {
      if (code === 0) resolveBuild();
      else reject(new Error(`cargo build exited ${code}\n${stderr}`));
    });
    child.on("error", reject);
  });
}

/** Spawn the demo server binary; resolve when it prints GWP_TCP_ADDR=. */
function startDemoApp(): Promise<{ addr: string; stop: () => void }> {
  const child = spawn(demoBin, ["--port", "0"], {
    stdio: ["ignore", "pipe", "inherit"],
  });
  const addr = new Promise<string>((resolveAddr, reject) => {
    let buffer = "";
    const timer = setTimeout(
      () => reject(new Error("demo app did not print GWP_TCP_ADDR= within 120 s")),
      120_000,
    );
    child.stdout.on("data", (chunk: Buffer) => {
      buffer += chunk.toString();
      const line = buffer.split("\n").find((l) => l.startsWith("GWP_TCP_ADDR="));
      if (line) {
        clearTimeout(timer);
        // The stdout pipe keeps the runner's event loop referenced after
        // the tests finish — once the address is ours, nothing more is
        // read, so destroy the pipe and let the file drain.
        child.stdout.destroy();
        resolveAddr(line.slice("GWP_TCP_ADDR=".length).trim());
      }
    });
    child.on("exit", () => {
      clearTimeout(timer);
      reject(new Error("demo app exited before printing its address"));
    });
  });
  return addr.then((addr) => ({
    addr,
    stop: () => {
      child.kill();
      child.stdout?.destroy();
    },
  }));
}

before(async () => {
  await buildBinaries();
  demo = await startDemoApp();
  workDir = mkdtempSync(join(tmpdir(), "gwp-e2e-full-stack-"));
});

after(() => {
  demo.stop();
});

test("§13 remote codegen: TS starts the recorder, acts, stops, gets the exact kit code", async () => {
  const client = await GwpClient.connectTcp(`127.0.0.1:${Number(demo.addr.split(":")[1])}`);
  try {
    const welcome = await client.handshake("ts-e2e-full-stack/0.1");
    assert.equal(welcome.adapter.name, "mock", "the demo server's adapter");

    await client.inspectStartRecording();

    // Click through the wire path (resolve → act), the same surface the
    // Rust recorder taps.
    const save = await client.resolve({
      kind: "role",
      role: "Button",
      name: { text: "Save", exact: false, ignore_case: true },
    });
    const clicked = await client.action(save.node, save.generation, { kind: "click" });
    assert.equal(clicked.detail, "clicked");

    const email = await client.resolve({
      kind: "role",
      role: "TextInput",
      name: { text: "Email", exact: false, ignore_case: true },
    });
    await client.action(email.node, email.generation, {
      kind: "set_value",
      text: "ada@lovelace.dev",
    });

    const { code, warnings } = await client.inspectStopRecording();
    assert.equal(
      code,
      'app.get_by_role(Role::Button, "Save").click();\n' +
        'app.get_by_role(Role::TextInput, "Email").fill("ada@lovelace.dev");\n',
      "the §13 cost model's recorded program — identical to the Rust client's",
    );
    assert.deepEqual(warnings, [], "strict mode stayed clean");
    await client.ping(); // the door stayed healthy through the recording
  } finally {
    await client.close();
  }
});

test("§13 double-stop is the documented clean rejection over the wire", async () => {
  const client = await GwpClient.connectTcp(`127.0.0.1:${Number(demo.addr.split(":")[1])}`);
  try {
    await client.handshake("ts-e2e-full-stack/0.1");
    await client.inspectStartRecording();
    await client.inspectStopRecording(); // drains the (empty) recording
    await assert.rejects(
      () => client.inspectStopRecording(),
      (err: unknown) => {
        assert.ok(err instanceof GwpRejected, "the typed wire rejection");
        assert.match(
          (err as GwpRejected).message,
          /no codegen recording is active/,
          "the domain error text",
        );
        return true;
      },
    );
  } finally {
    await client.close();
  }
});

test("§8.1 TS filter(): the composed rows resolve against the real server (m34/m91)", async () => {
  const client = await GwpClient.connectTcp(demo.addr);
  try {
    await client.handshake("ts-e2e-full-stack/0.1");

    // The positive filter through the helper: the group whose subtree
    // holds a button — the ancestor row rides the m91 appended
    // `SelectorV1::Has` variant over the real door.
    const group = await client.resolve(
      filter(
        { kind: "role", role: "Group", name: null },
        { has: { kind: "role", role: "Button", name: null } },
      ),
    );
    assert.equal(group.node.kind, "accesskit", "the group resolves");

    // The m34 filter row through the helper: the Save button narrowed
    // by the string form (the fuzzy default) and the document-order
    // pick — the row the wire already spoke, now typed client-side.
    const save = await client.resolve(
      filter({ kind: "role", role: "Button", name: null }, {
        hasText: "Save",
        nth: "first",
      }),
    );
    const clicked = await client.action(save.node, save.generation, { kind: "click" });
    assert.equal(clicked.detail, "clicked", "the has-filtered resolve acts");
  } finally {
    await client.close();
  }
});

test("§8.1 TS regex rows: the pattern probes resolve and act over the real door (m95)", async () => {
  const client = await GwpClient.connectTcp(demo.addr);
  try {
    await client.handshake("ts-e2e-full-stack/0.1");

    // The role-name regex row (the typed builder, the /re/ flags folded
    // inline): the role bucket narrows by the pattern over the real
    // wire — the set read answers both buttons.
    const buttons = await client.resolveAll(roleNameRegex("Button", /^(Save|Cancel)$/));
    assert.equal(buttons.length, 2, "Save and Cancel answer the pattern");

    // The text probe through its own row: the anchored form answers one.
    const save = await client.resolve(textRegex(/^Save$/));
    assert.equal(save.node.kind, "accesskit", "the anchored probe resolves Save");

    // The regex filter through the helper: /Sav/ narrows the role set —
    // the appended row over the door — and the action rides it.
    const narrowed = await client.resolve(
      filter({ kind: "role", role: "Button", name: null }, { hasText: /Sav/ }),
    );
    const done = await client.action(narrowed.node, narrowed.generation, { kind: "click" });
    assert.equal(done.detail, "clicked", "the regex-filtered resolve acts");

    // The honest empty read: a pattern nothing carries answers zero —
    // the total read, never an error.
    const ghosts = await client.resolveAll(labelRegex(/Ghost/));
    assert.equal(ghosts.length, 0);
  } finally {
    await client.close();
  }
});

test("§8.1 TS getBy* factories: the kit's sugar mirror resolves and acts over the real door (m96)", async () => {
  const client = await GwpClient.connectTcp(demo.addr);
  try {
    await client.handshake("ts-e2e-full-stack/0.1");

    // The named role factory (the kit's get_by_role mirror): the fuzzy
    // default resolves Save through the auto-waited strict read, and
    // the action rides the same aim.
    const save = await client.resolve(getByRole("Button", { name: "Save" }));
    const done = await client.action(save.node, save.generation, { kind: "click" });
    assert.equal(done.detail, "clicked", "getByRole's row aims and acts");

    // The exact form beside it: case-sensitive whole-string — a wrong
    // case stays pending, so the honest read is the unnamed role set.
    const exact = await client.resolve(
      getByRole("Button", { name: "Save", exact: true }),
    );
    assert.equal(exact.node.kind, "accesskit", "the exact form resolves too");

    // The unnamed role factory (the kit's role_locator shape): the
    // whole bucket — the fixture's five buttons, document order.
    const buttons = await client.resolveAll(getByRole("Button"));
    assert.equal(buttons.length, 5, "the fixture's five buttons answer");

    // The text/label/value factories: the string rows and the regex
    // rows over the same door — the label first (the source order),
    // the form's value, the pattern probe.
    const label = await client.resolve(getByLabel("Welcome, Ada"));
    assert.equal(label.node.kind, "accesskit", "the label row resolves");
    const text = await client.resolve(getByText(/Welcome/));
    assert.equal(text.node.kind, "accesskit", "the text regex row resolves");
    const input = await client.resolve(getByValue("ada@example.dev"));
    assert.equal(input.node.kind, "accesskit", "the value row resolves");

    // The test-id factory's row rides the O(1) hash probe (§8.1) —
    // this mock door carries no test ids (they ride the logical-tree
    // path), so the set read answers the honest empty set: the read is
    // total, never an error.
    const ghosts = await client.resolveAll(getByTestId("btn-save"));
    assert.equal(ghosts.length, 0, "the door carries no test ids");
    const noLabel = await client.resolveAll(getByLabel(/Ghost/));
    assert.equal(noLabel.length, 0);
  } finally {
    await client.close();
  }
});

test("§12 live trace HTML: the Rust emitter captures the TS client's session", async () => {
  const out = join(workDir, "guiwright-live.html");
  const emitter = spawn(
    cliBin,
    [
      "live-html",
      `tcp://${demo.addr}`,
      "--out",
      out,
      "--duration-ms",
      "4000",
      "--test",
      "ts-e2e-live",
    ],
    { stdio: ["ignore", "inherit", "inherit"] },
  );
  const exited = new Promise<number>((resolveExit) => emitter.on("exit", resolveExit));

  // The TypeScript client acts inside the capture window: one real click
  // the feed's tree re-sync reflects. One session thread serves every
  // connection (§5.1), so the emitter sees the same session.
  console.log("live: connecting the TS driver client");
  const client = await GwpClient.connectTcp(`127.0.0.1:${Number(demo.addr.split(":")[1])}`);
  try {
    await client.handshake("ts-e2e-live-driver/0.1");
    console.log("live: handshaked — clicking Cancel inside the window");
    const cancel = await client.resolve({
      kind: "role",
      role: "Button",
      name: { text: "Cancel", exact: false, ignore_case: true },
    });
    await client.action(cancel.node, cancel.generation, { kind: "click" });
    console.log("live: click applied — closing the driver client");
  } finally {
    await client.close();
  }
  console.log("live: polling for the scrubber HTML");

  // The emitter writes the scrubber HTML mid-session (window end or the
  // quiet-stretch early end) — poll for the file, then assert the stamps.
  const deadline = Date.now() + 30_000;
  while (!existsSync(out) && Date.now() < deadline) {
    await new Promise((r) => setTimeout(r, 100));
  }
  console.log("live: awaiting the emitter's exit");
  const exitCode = await exited;
  console.log("live: emitter exited", exitCode);
  assert.equal(exitCode, 0, "the emitter exits 0");
  assert.ok(existsSync(out), "the live HTML was written");

  const html = readFileSync(out, "utf8");
  assert.ok(html.startsWith("<!doctype html>"), "the self-contained template");
  assert.ok(html.includes("gwtrace/v1 (live)"), "the live schema stamp (§12)");
  assert.ok(html.includes('"adapter":"mock"'), "the adapter header");
  assert.ok(html.includes("ts-e2e-live"), "the test label override");
  // The captured tree: the fixture's buttons ride the attach dump and
  // the post-click re-sync — the same assertions the Rust live_html E2E
  // pins (crates/guiwright-cli/tests/live_html.rs).
  assert.ok(html.includes("Save"), "the captured tree rides the payload");
  assert.ok(html.includes("Cancel"), "the full dump is in the payload");
});
