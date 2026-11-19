/**
 * The client-side content-assertion evaluator (M54 — the kit's `expect`
 * vocabulary's content half over any door): the pure `ExpectTracker`
 * pinned without a wire, the same cases
 * `crates/guiwright-proto/tests/expect_tracker.rs` pins row-for-row —
 * satisfied-on-entry, pending-on-empty, the m29/m31 source orders (label
 * first vs value first, a value never satisfying the other read
 * silently), the Playwright text-match semantics (substring +
 * case-insensitive + whitespace-normalized unless `exact`), and the
 * mismatch reason naming what the last evaluation found.
 */

import { test } from "node:test";
import assert from "node:assert/strict";

import { ExpectTracker, expectState, textMatches, type ExpectKind } from "../src/expect.ts";
import type { DescribedNode, NodeKey, TextMatchV1 } from "../src/schema.ts";

/** The 16-byte AccessKit tree id, its hex wire/test form. */
const TREE = "a6000000deadbeefa6000000deadbeef";

function node(id: number): NodeKey {
  return { kind: "accesskit", tree: TREE, node: id };
}

/** The fixture's Email input row shape: name "Email", value
 * "ada@example.dev" — the pair the m29/m31 orders were pinned against. */
function email(): DescribedNode {
  return {
    node: node(5),
    generation: 0,
    role: "TextInput",
    name: "Email",
    value: "ada@example.dev",
    bounds: [16, 128, 256, 156],
  };
}

function row(id: number, name: string | null, value: string | null): DescribedNode {
  return { node: node(id), generation: 0, role: "TextInput", name, value, bounds: null };
}

function fuzzy(pattern: string): TextMatchV1 {
  return { text: pattern, exact: false, ignore_case: true };
}

function exact(pattern: string): TextMatchV1 {
  return { text: pattern, exact: true, ignore_case: false };
}

test("text assertion reads the label first and hands back the row", () => {
  const tracker = new ExpectTracker("text", fuzzy("Email"));
  const answer = tracker.evaluate([email()]);
  assert.equal(answer.done, true);
  if (answer.done) {
    assert.equal(answer.row.name, "Email");
    assert.equal(answer.row.value, "ada@example.dev");
  }
});

test("a value never satisfies the text read while the label rides", () => {
  const tracker = new ExpectTracker("text", fuzzy("ada@example.dev"));
  const answer = tracker.evaluate([email()]);
  assert.equal(answer.done, false);
  if (!answer.done) assert.match(answer.reason, /expected/);
});

test("value assertion reads the value first; the label never satisfies it", () => {
  const value = new ExpectTracker("value", fuzzy("ada@example.dev"));
  assert.equal(value.evaluate([email()]).done, true);

  const wrong = new ExpectTracker("value", fuzzy("Email"));
  assert.equal(wrong.evaluate([email()]).done, false);
});

test("the fallback order holds at the row edges; a bare row stays pending", () => {
  const nameless = row(6, null, "ada@example.dev");
  const text = new ExpectTracker("text", fuzzy("ada@example.dev"));
  assert.equal(text.evaluate([nameless]).done, true);

  const valueless = row(4, "Welcome, Ada", null);
  const value = new ExpectTracker("value", fuzzy("Welcome, Ada"));
  assert.equal(value.evaluate([valueless]).done, true);

  const bare = row(7, null, null);
  assert.equal(text.evaluate([bare]).done, false);
  assert.equal(value.evaluate([bare]).done, false);
});

test("the match is Playwright fuzzy until exact", () => {
  const tracker = new ExpectTracker("text", fuzzy("welcome, ada"));
  assert.equal(tracker.evaluate([row(4, "Welcome,  Ada", null)]).done, true);

  const exactTracker = new ExpectTracker("text", exact("Welcome, Ada"));
  assert.equal(exactTracker.evaluate([row(4, "welcome, ada", null)]).done, false);
  assert.equal(exactTracker.evaluate([row(4, "Welcome, Ada and friends", null)]).done, false);
  assert.equal(exactTracker.evaluate([row(4, "Welcome,   Ada", null)]).done, true);
});

test("the set head pins the assertion; later rows never answer", () => {
  const tracker = new ExpectTracker("text", fuzzy("Cancel"));
  const rows = [row(2, "Save", null), row(3, "Cancel", null)];
  const answer = tracker.evaluate(rows);
  assert.equal(answer.done, false);
  if (!answer.done) assert.match(answer.reason, /Save/);
});

test("an empty set stays pending, naming the miss", () => {
  const tracker = new ExpectTracker("text", fuzzy("Save"));
  const answer = tracker.evaluate([]);
  assert.equal(answer.done, false);
  if (!answer.done) assert.match(answer.reason, /no match/);
});

test("the mismatch reason names expected and found", () => {
  const tracker = new ExpectTracker("text", exact("Done"));
  const answer = tracker.evaluate([row(2, "Save", null)]);
  assert.equal(answer.done, false);
  if (!answer.done) {
    assert.match(answer.reason, /Save/);
    assert.match(answer.reason, /expected/);
  }
});

test("the kind state words are stable (the timeout's static word)", () => {
  const kinds: ExpectKind[] = ["text", "value", "role", "name"];
  assert.deepEqual(kinds.map(expectState), ["have text", "have value", "have role", "have name"]);
});

test("the matcher helper mirrors the core TextMatch rule", () => {
  assert.equal(textMatches(fuzzy("ADA"), "  ada   example  "), true);
  assert.equal(textMatches(exact("ada"), "Ada"), false);
  assert.equal(textMatches(exact("ada"), "ada"), true);
  assert.equal(textMatches({ text: "ad", exact: false, ignore_case: false }, "ada"), true);
});

// --- the identity kinds (m62/m63 — the m38 vocabulary over the door) -------

test("the role assertion answers the head row's role exactly", () => {
  const tracker = new ExpectTracker("role", "TextInput");
  const answer = tracker.evaluate([email()]);
  assert.equal(answer.done, true);
  if (answer.done) {
    assert.equal(answer.row.role, "TextInput", "the structural fact the assertion speaks");
    assert.equal(answer.row.name, "Email", "the row itself rides back (the dump-row rule)");
  }
});

test("the role compares exactly — a structural fact takes no fuzz", () => {
  const tracker = new ExpectTracker("role", "textinput");
  const answer = tracker.evaluate([email()]);
  assert.equal(answer.done, false);
  if (!answer.done) {
    assert.match(answer.reason, /textinput/);
    assert.match(answer.reason, /TextInput/);
  }
});

test("the role mismatch reason names both sides", () => {
  const button = row(2, "Save", null);
  button.role = "Button";
  const tracker = new ExpectTracker("role", "TextInput");
  const answer = tracker.evaluate([button]);
  assert.deepEqual(answer, {
    done: false,
    reason: 'role is "Button", expected "TextInput"',
  });
});

test("the name assertion reads the label only — the value never answers", () => {
  const tracker = new ExpectTracker("name", fuzzy("ada@example.dev"));
  const answer = tracker.evaluate([email()]);
  assert.equal(answer.done, false, "the m62 narrower read: the m29 order never rides here");
  if (!answer.done) {
    assert.match(answer.reason, /name is "Email"/);
    assert.match(answer.reason, /expected/);
  }

  const satisfied = new ExpectTracker("name", fuzzy("Email"));
  const hit = satisfied.evaluate([email()]);
  assert.equal(hit.done, true);
});

test("a node with no accessible name stays pending, naming the missing fact", () => {
  const tracker = new ExpectTracker("name", fuzzy("Editor"));
  const answer = tracker.evaluate([row(4, null, "ada@example.dev")]);
  assert.equal(answer.done, false);
  if (!answer.done) assert.match(answer.reason, /no accessible name/);
});

test("the identity rows pin the set's head; later rows never answer", () => {
  const cancel = row(3, "Cancel", null);
  cancel.role = "Button";
  const save = row(2, "Save", null);
  save.role = "Button";
  const tracker = new ExpectTracker("role", "TextInput");
  const answer = tracker.evaluate([save, cancel]);
  assert.equal(answer.done, false, "the second row's role would satisfy — the head pins");
  if (!answer.done) {
    assert.match(answer.reason, /role is "Button"/, "the head row's fact");
    assert.match(answer.reason, /expected "TextInput"/);
  }
});
