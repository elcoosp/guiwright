/**
 * The TS regex text rows' consumer demo (the m95 regex family's TS
 * half — the typed builders shipped with it: `textRegex`,
 * `labelRegex`, `valueRegex`, `roleNameRegex`, `hasTextRegex`, the
 * `RegExp` acceptance folding the wire-readable flags inline and
 * refusing the rest): the probes resolve through the same strict,
 * auto-waited reads the string rows ride — here `describeAll()` (the
 * set's facts), `resolve()` (the strict aim) and the
 * `filter({ hasText: /re/ })` composition — no locator object, no
 * wire change beyond the appended rows.
 *
 * ```text
 * npm run example:regex
 * # spawns the Rust demo app on the default state fixture, then prints
 * # the case-folded Save+Cancel set, the anchored role-name aim, the
 * # honest empty for an anchored case-sensitive probe, the Email
 * # value probe, the label probe, the RegExp hasText narrowing and
 * # the typed refusal for a flag the wire cannot speak
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
import {
  filter,
  textRegex,
  roleNameRegex,
  valueRegex,
  labelRegex,
  toRegexPattern,
} from "../src/filter.ts";

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
    await client.handshake("ts-regex-demo/0.1");

    // The text probe (the `getByText(/re/)` row): the JS-convention
    // semantics — partial by default, the inline flags deciding case —
    // so the case-folded alternation answers both buttons in document
    // order.
    const rows = await client.describeAll(textRegex(/save|cancel/i));
    assert.deepEqual(
      rows.map((r) => r.name),
      ["Save", "Cancel"],
      "the case-folded probe answers both buttons in document order",
    );
    console.log(`text    → /save|cancel/i answered ${rows.map((r) => r.name).join(", ")}`);

    // The role-name probe (the `getByRole(role, { name: /re/ })` row):
    // the role bucket narrows by the pattern, strict mode stays on —
    // the anchored form aims exactly the Save button.
    const save = await client.resolve(roleNameRegex("Button", /^Save$/));
    assert.equal(save.node.kind, "accesskit", "the demo app speaks AccessKit handles");
    assert.equal(save.node.node, 2, "the anchored role-name probe aims the Save button");
    console.log(`role    → Button, name /^Save$/ aimed node ${save.node.node} (strict mode held)`);

    // The anchoring honesty: an anchored case-sensitive probe the set
    // cannot satisfy answers the honest empty — no fuzzy fallback.
    const none = await client.resolveAll(textRegex("^save$"));
    assert.equal(none.length, 0, "the anchored case-sensitive probe answers the empty set");
    console.log("anchor  → /^save$/ answered the honest empty set");

    // The value probe (the `getByValue(/re/)` row): the Email input's
    // value speaks the pattern.
    const email = await client.describeAll(valueRegex(/^\S+@\S+\.\S+$/));
    assert.equal(email.length, 1, "the value probe aims the Email input");
    assert.equal(email[0].value, "ada@example.dev", "the value the probe matched");
    console.log(`value   → /^\\S+@\\S+\\.\\S+$/ matched the input carrying ${email[0].value}`);

    // The label probe (the `getByLabel(/re/)` row): the Welcome
    // label's accessible name speaks the pattern.
    const label = await client.describeAll(labelRegex(/^Wel.* Ada$/));
    assert.deepEqual(label.map((r) => r.name), ["Welcome, Ada"], "the label probe aims the label");
    console.log(`label   → /^Wel.* Ada$/ aimed "${label[0].name}"`);

    // The RegExp hasText composition (the m95 family beside the m34
    // string filter form): the narrowing rides the appended
    // `has_text_regex` row as the composed row's own source probe.
    const narrowed = await client.describeAll(
      filter({ kind: "role", role: "Button", name: null }, { hasText: /Sa/ }),
    );
    assert.deepEqual(
      narrowed.map((r) => r.name),
      ["Save"],
      "the RegExp hasText narrowing keeps Save alone",
    );
    console.log(
      `filter  → hasText: /Sa/ narrowed the buttons to ${narrowed.map((r) => r.name).join(", ")}`,
    );

    // The typed refusal: a matching-mode flag has no wire reading —
    // the fold names it instead of silently dropping semantics.
    try {
      toRegexPattern(/save/g);
      assert.fail("the g flag must be refused");
    } catch (err) {
      assert.match((err as Error).message, /flags \[g\] have no wire reading/);
      console.log(`refusal → ${(err as Error).message}`);
    }
  } finally {
    await client.close();
  }
} finally {
  demo.stop();
}
