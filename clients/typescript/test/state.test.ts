/**
 * The state-assertion evaluator pins (m75 — the m73 Rust tracker pins
 * mirrored row-for-row): the pure `StateTracker` over the identity-state
 * rows the m70 `describeAllIdentityState()` read answers — the checked
 * four-way word demanded exactly (a mixed checkbox is not an unchecked
 * one, a not-checkable role is the total fact), the enabled boolean both
 * ways, the focused fact, the head pins and the empty set stays pending
 * on every kind.
 */

import { test } from "node:test";
import assert from "node:assert/strict";

import { StateTracker, stateWord } from "../src/expect.ts";
import type { CheckedState, DescribedIdentityStateNode, NodeKey } from "../src/schema.ts";

function logical(id: number): NodeKey {
  return { kind: "logical", adapter: "test", id };
}

function stateRow(
  id: number,
  name: string,
  state: { focused?: boolean; enabled?: boolean; checked?: CheckedState } = {},
): DescribedIdentityStateNode {
  return {
    node: logical(id),
    generation: 0,
    role: "CheckBox",
    name,
    value: null,
    bounds: [16, 204, 136, 236],
    description: null,
    focused: state.focused ?? false,
    enabled: state.enabled ?? true,
    checked: state.checked ?? "unchecked",
  };
}

test("checked assertion answers the exact four-way word", () => {
  const tracker = new StateTracker("checked", "checked");
  const live = stateRow(9, "Notifications", { checked: "checked" });
  const answer = tracker.evaluate([live]);
  assert.ok(answer.done, "the checked word satisfies the checked demand");
  assert.equal(answer.row.checked, "checked", "the fact rides the row");

  // The tri-state demanded exactly: the mixed word never satisfies an
  // unchecked demand and vice versa (the m73 rule).
  const mixed = new StateTracker("checked", "mixed");
  assert.ok(mixed.evaluate([live]).done === false, "mixed never answers checked");

  const dark = stateRow(10, "Dark mode", { checked: "unchecked" });
  const unchecked = new StateTracker("checked", "unchecked");
  assert.ok(unchecked.evaluate([dark]).done, "the unchecked word answers");
  assert.ok(
    unchecked.evaluate([live]).done === false,
    "a checked checkbox is not an unchecked one",
  );
});

test("checked demand treats not-checkable as the total fact, pending", () => {
  // The m68 word demanded as the word: a not-checkable row stays
  // pending under a checked demand — a fact, never an error.
  const button = stateRow(2, "Save", { checked: "not_checkable" });
  const tracker = new StateTracker("checked", "checked");
  const answer = tracker.evaluate([button]);
  assert.ok(answer.done === false, "the not-checkable fact stays pending");
  assert.match(answer.reason, /checked is "not_checkable"/);
});

test("enabled assertion answers the boolean both ways", () => {
  const live = stateRow(9, "Notifications", { enabled: true });
  const enabled = new StateTracker("enabled", true);
  assert.ok(enabled.evaluate([live]).done, "the enabled fact answers");
  const disabled = new StateTracker("enabled", false);
  const negative = disabled.evaluate([live]);
  assert.ok(negative.done === false, "the honest negative stays pending");
  assert.match(negative.reason, /enabled is true, expected false/);

  const dead = stateRow(7, "Confirm", { enabled: false, checked: "not_checkable" });
  assert.ok(disabled.evaluate([dead]).done, "the disabled fact answers");
});

test("focused assertion waits for the focus to arrive", () => {
  const tracker = new StateTracker("focused");
  const row = stateRow(9, "Notifications", { focused: true });
  const answer = tracker.evaluate([row]);
  assert.ok(answer.done, "the focused fact answers");
  assert.ok(answer.row.focused, "the satisfying row carries the fact");

  const unfocused = tracker.evaluate([stateRow(9, "Notifications")]);
  assert.ok(unfocused.done === false, "an unfocused node stays pending");
  assert.match(unfocused.reason, /focused is false, expected true/);
});

test("empty set stays pending on every kind", () => {
  const kinds = [
    new StateTracker("checked", "checked"),
    new StateTracker("checked", "unchecked"),
    new StateTracker("enabled", true),
    new StateTracker("focused"),
  ];
  for (const tracker of kinds) {
    const answer = tracker.evaluate([]);
    assert.ok(answer.done === false, "no node, no fact, no satisfaction");
    assert.equal(answer.reason, "no match yet (the set is empty)");
  }
});

test("the timeout words name the to_be_* vocabulary", () => {
  assert.equal(stateWord("checked"), "be checked");
  assert.equal(stateWord("enabled"), "be enabled");
  assert.equal(stateWord("focused"), "be focused");
});
