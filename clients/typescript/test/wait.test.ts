/**
 * The client-side wait evaluator's state machine (M45 — the m42 seam's
 * TypeScript row): the pure `WaitTracker` pinned without a wire, the
 * same cases `crates/guiwright-proto/tests/wait_tracker.rs` pins
 * row-for-row — attached-by-any-row (bounds-less included),
 * pending-on-empty, visible's bounds + area guard, hidden's absence,
 * stable's baseline/confirm pair, the bounds-change reset and the
 * identity-change re-pin, and the selector render.
 */

import { test } from "node:test";
import assert from "node:assert/strict";

import { renderSelector, WaitTracker, type WaitState } from "../src/wait.ts";
import type { DescribedNode, NodeKey } from "../src/schema.ts";

/** The fixture's Save button rect (16,16 → 136,48, physical px). */
const SAVE_BOUNDS: [number, number, number, number] = [16, 16, 136, 48];

/** The 16-byte AccessKit tree id, its hex wire/test form. */
const TREE = "a6000000deadbeefa6000000deadbeef";

function node(id: number): NodeKey {
  return { kind: "accesskit", tree: TREE, node: id };
}

function row(id: number, name: string, bounds: DescribedNode["bounds"]): DescribedNode {
  return {
    node: node(id),
    generation: 0,
    role: "Button",
    name,
    value: null,
    bounds,
  };
}

test("attached is satisfied by any row, even a boundsless one", () => {
  // The hidden "Invisible" button (node 6, no bounds) is still attached:
  // Playwright's attached-vs-visible distinction over the wire rows.
  const tracker = new WaitTracker("attached" satisfies WaitState);
  const rows = [row(6, "Invisible", null)];
  const answer = tracker.evaluate(rows);
  assert.deepEqual(answer, { done: true, row: rows[0] });
});

test("attached stays pending on an empty set", () => {
  const tracker = new WaitTracker("attached");
  const answer = tracker.evaluate([]);
  assert.equal(answer.done, false);
  assert.match(answer.done ? "" : answer.reason, /no match/);
});

test("visible requires non-empty bounds", () => {
  const tracker = new WaitTracker("visible");
  const hiddenOnly = tracker.evaluate([row(6, "Invisible", null)]);
  assert.equal(hiddenOnly.done, false);
  assert.match(hiddenOnly.done ? "" : hiddenOnly.reason, /no bounds/);

  const visible = [row(2, "Save", SAVE_BOUNDS)];
  assert.deepEqual(tracker.evaluate(visible), { done: true, row: visible[0] });
});

test("visible rejects a zero-area rect", () => {
  // A degenerate rect ([16,16 → 16,48]: zero width) is not laid out —
  // the §8.3 area guard, client-side.
  const tracker = new WaitTracker("visible");
  const answer = tracker.evaluate([row(2, "Save", [16, 16, 16, 48])]);
  assert.equal(answer.done, false);
});

test("hidden is satisfied by absence of any visible row", () => {
  // Satisfied by absence too (Playwright parity): no rows at all, or
  // only bounds-less rows.
  const tracker = new WaitTracker("hidden");
  assert.deepEqual(tracker.evaluate([]), { done: true, row: null });
  assert.deepEqual(tracker.evaluate([row(6, "Invisible", null)]), {
    done: true,
    row: null,
  });

  // A visible row keeps the wait pending.
  const pending = tracker.evaluate([row(2, "Save", SAVE_BOUNDS)]);
  assert.equal(pending.done, false);
});

test("stable needs two consecutive identical frame observations", () => {
  // The §8.3 stable semantics: the tracker is frame-fed only — a tree
  // wake re-pins but never advances the count — and two consecutive
  // identical bounds across presented frames confirm.
  const tracker = new WaitTracker("stable");
  const rows = [row(2, "Save", SAVE_BOUNDS)];

  // Entry evaluation: the visible precondition holds, but no frame has
  // confirmed anything yet.
  assert.equal(tracker.evaluate(rows).done, false);
  // First presented frame: the bounds become the baseline, not a pass.
  assert.equal(tracker.observeFrame(rows).done, false);
  // Second identical frame: confirmed.
  assert.deepEqual(tracker.observeFrame(rows), { done: true, row: rows[0] });
});

test("stable resets when the bounds change", () => {
  const tracker = new WaitTracker("stable");
  const save = [row(2, "Save", SAVE_BOUNDS)];
  assert.equal(tracker.evaluate(save).done, false);
  assert.equal(tracker.observeFrame(save).done, false);
  assert.equal(tracker.observeFrame(save).done, true);

  // The node moves: the changed frame becomes the new baseline (the
  // core rule) — the old confirmation says nothing about the moved
  // rect, and the moved rect needs its own baseline + confirm pair.
  const moved = [row(2, "Save", [16, 96, 136, 128])];
  const reset = tracker.observeFrame(moved);
  assert.equal(reset.done, false);
  assert.match(reset.done ? "" : reset.reason, /next frame/);
  assert.deepEqual(tracker.observeFrame(moved), {
    done: true,
    row: moved[0],
  });
});

test("stable repins when the pinned row leaves the set", () => {
  // A waiter resets on identity change (§8.2): the pinned node gone
  // re-pins to the set's visible row and the count starts over.
  const tracker = new WaitTracker("stable");
  const save = [row(2, "Save", SAVE_BOUNDS)];
  assert.equal(tracker.evaluate(save).done, false);
  assert.equal(tracker.observeFrame(save).done, false);

  // The set now leads with a different visible node (node 3 "Cancel").
  const changed = [row(3, "Cancel", [16, 56, 136, 88]), row(2, "Save", SAVE_BOUNDS)];
  const rePin = tracker.observeFrame(changed);
  assert.equal(rePin.done, false);
  assert.match(rePin.done ? "" : rePin.reason, /re-pinned/);

  // One identical frame on the new pin is a baseline, not a pass — the
  // reset is real, not cosmetic.
  const cancelOnly = [row(3, "Cancel", [16, 56, 136, 88])];
  assert.equal(tracker.observeFrame(cancelOnly).done, false);
  assert.deepEqual(tracker.observeFrame(cancelOnly), {
    done: true,
    row: cancelOnly[0],
  });
});

test("stable stays pending without a visible row", () => {
  const tracker = new WaitTracker("stable");
  const hiddenOnly = [row(6, "Invisible", null)];
  const answer = tracker.evaluate(hiddenOnly);
  assert.equal(answer.done, false);
  assert.match(answer.done ? "" : answer.reason, /no visible/);
  assert.equal(tracker.observeFrame(hiddenOnly).done, false);
});

test("the selector render speaks the kit's vocabulary", () => {
  // The timeout error's locator line: exact matches quote, fuzzy
  // matches say so (the core crate's `Selector::render` shape).
  assert.equal(
    renderSelector({ kind: "role", role: "Button", name: null }),
    'getByRole("Button")',
  );
  assert.equal(
    renderSelector({
      kind: "role",
      role: "Button",
      name: { text: "Save", exact: true, ignore_case: false },
    }),
    'getByRole("Button", "Save")',
  );
  assert.equal(
    renderSelector({
      kind: "text",
      match: { text: "Welcome", exact: false, ignore_case: true },
    }),
    'getByText("Welcome" (fuzzy))',
  );
});
