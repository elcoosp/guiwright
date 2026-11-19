/**
 * The description-assertion evaluator pins (m65 — the m64 Rust
 * tracker pins mirrored row-for-row): the pure `ExpectTracker` over the
 * identity rows the appended `describeAllIdentity()` read answers —
 * the description fact answers, the name and the value never do, the
 * mismatch names both sides, the head pins and the empty set stays
 * pending.
 */

import { test } from "node:test";
import assert from "node:assert/strict";

import { ExpectTracker, expectState } from "../src/expect.ts";
import type { DescribedIdentityNode, NodeKey, TextMatchV1 } from "../src/schema.ts";

function logical(id: number): NodeKey {
  return { kind: "logical", adapter: "test", id };
}

function identityRow(
  id: number,
  name: string | null,
  description: string | null,
): DescribedIdentityNode {
  return {
    node: logical(id),
    generation: 0,
    role: "Button",
    name,
    value: null,
    bounds: [16, 204, 136, 236],
    description,
  };
}

function fuzzy(pattern: string): TextMatchV1 {
  return { text: pattern, exact: false, ignore_case: true };
}

function exact(pattern: string): TextMatchV1 {
  return { text: pattern, exact: true, ignore_case: false };
}

test("description assertion reads the description fact", () => {
  const tracker = new ExpectTracker("description", fuzzy("Installs"));
  const install = identityRow(9, "Install", "Installs the latest build");
  const answer = tracker.evaluate([install]);
  assert.ok(answer.done, "the description satisfies the description read");
  assert.equal(answer.row.node, install.node, "the head row answers");
  assert.equal(
    (answer.row as DescribedIdentityNode).description,
    "Installs the latest build",
    "the fact rides the row",
  );
});

test("description assertion never takes the name or value", () => {
  const tracker = new ExpectTracker("description", fuzzy("Save"));
  const save = identityRow(2, "Save", null);
  const answer = tracker.evaluate([save]);
  assert.ok(!answer.done);
  assert.match(answer.reason, /no accessible description/, "the missing fact is named");
});

test("the description mismatch names expected and found", () => {
  const tracker = new ExpectTracker("description", exact("Ghost"));
  const install = identityRow(9, "Install", "Installs the latest build");
  const answer = tracker.evaluate([install]);
  assert.ok(!answer.done);
  assert.match(answer.reason, /Installs the latest build/, "found side");
  assert.match(answer.reason, /expected/, "wanted side");
});

test("the identity rows pin the set head; an empty set stays pending", () => {
  const tracker = new ExpectTracker("description", fuzzy("Installs"));
  const rows = [identityRow(2, "Save", null), identityRow(9, "Install", "Installs the latest build")];
  const answer = tracker.evaluate(rows);
  assert.ok(!answer.done, "the second row must not answer");
  assert.match(answer.reason, /no accessible description/, "the head is the pin");
  const empty = tracker.evaluate([]);
  assert.ok(!empty.done);
  assert.match(empty.reason, /no match/, "an empty set cannot satisfy");
});

test("the state word the description timeout names is stable", () => {
  assert.equal(expectState("description"), "have description");
});
