/**
 * The client-side count-assertion evaluator (m59/m60 — the kit's
 * `expect` vocabulary's count half over any door): the pure
 * `CountTracker` pinned without a wire, the same cases the Rust
 * `crates/guiwright-proto/src/count.rs` module pins row-for-row — the
 * exact form answering the set length, absence as an answer (an empty
 * set is found 0: satisfying `count 0` and every less-than, pending a
 * greater-than), the comparators answering the same found fact, and the
 * mismatch reason naming what the last evaluation found.
 */

import { test } from "node:test";
import assert from "node:assert/strict";

import { CountTracker, HAVE_COUNT_STATE, type CountCompare } from "../src/count.ts";

/** Resolve-all row shapes — a count is a fact about the rows' length,
 * never their contents. */
const BUTTONS = [{}, {}, {}, {}];
const TWO = [{}, {}];

function compare(kind: CountCompare["kind"], bound: number): CountCompare {
  return { kind, bound };
}

test("the exact form answers the set length", () => {
  const tracker = new CountTracker(compare("exact", 4));
  const answer = tracker.evaluate(BUTTONS);
  assert.deepEqual(answer, { done: true, found: 4 });
});

test("the mismatch names both sides", () => {
  const tracker = new CountTracker(compare("exact", 4));
  const answer = tracker.evaluate(TWO);
  assert.deepEqual(answer, {
    done: false,
    reason: "count is 2, expected 4",
  });
});

test("an empty set is found zero — absence is an answer", () => {
  const exact = new CountTracker(compare("exact", 0));
  assert.deepEqual(exact.evaluate([]), { done: true, found: 0 });
  const under = new CountTracker(compare("less_than", 3));
  assert.deepEqual(under.evaluate([]), { done: true, found: 0 });
  const over = new CountTracker(compare("greater_than", 0));
  assert.deepEqual(over.evaluate([]), {
    done: false,
    reason: "count is 0, expected > 0",
  });
});

test("the comparators answer the same found fact", () => {
  const more = new CountTracker(compare("greater_than", 3));
  assert.deepEqual(more.evaluate(BUTTONS), { done: true, found: 4 });
  assert.deepEqual(more.evaluate(TWO), {
    done: false,
    reason: "count is 2, expected > 3",
  });
  const fewer = new CountTracker(compare("less_than", 5));
  assert.deepEqual(fewer.evaluate(BUTTONS), { done: true, found: 4 });
  assert.deepEqual(fewer.evaluate([{}, {}, {}, {}, {}]), {
    done: false,
    reason: "count is 5, expected < 5",
  });
});

test("the inclusive comparators hold on the bound", () => {
  // The m25 reserved pair (`at_least`/`at_most`) mirrored row for row
  // from the Rust tracker's `the_inclusive_comparators_hold_on_the_bound`
  // pin (crates/guiwright-proto/src/count.rs): the bound itself holds —
  // `>= 4` on 4, `<= 4` on 4 — and the mismatches name the inclusive
  // form the kit's headers print (`count >= 4` / `count <= 4`).
  const least = new CountTracker(compare("at_least", 4));
  assert.deepEqual(least.evaluate(BUTTONS), { done: true, found: 4 });
  assert.deepEqual(least.evaluate([]), {
    done: false,
    reason: "count is 0, expected >= 4",
  });
  assert.deepEqual(least.evaluate(TWO), {
    done: false,
    reason: "count is 2, expected >= 4",
  });
  const most = new CountTracker(compare("at_most", 4));
  assert.deepEqual(most.evaluate(BUTTONS), { done: true, found: 4 });
  // Absence is an answer: an empty set satisfies every at-most.
  assert.deepEqual(most.evaluate([]), { done: true, found: 0 });
  assert.deepEqual(most.evaluate([{}, {}, {}, {}, {}]), {
    done: false,
    reason: "count is 5, expected <= 4",
  });
});

test("the range comparator holds within the bounds", () => {
  // The m25 honest list's range shape mirrored row for row from the
  // Rust tracker's `the_range_comparator_holds_within_the_bounds` pin
  // (crates/guiwright-proto/src/count.rs): both ends hold on the bound,
  // the inside answers, and the mismatch names the demanded range the
  // kit's headers print (`count 3..=5`).
  const range = new CountTracker({ kind: "between", min: 3, max: 5 });
  assert.deepEqual(range.evaluate(BUTTONS), { done: true, found: 4 });
  assert.deepEqual(range.evaluate([{}, {}, {}]), { done: true, found: 3 });
  assert.deepEqual(range.evaluate([]), {
    done: false,
    reason: "count is 0, expected 3..=5",
  });
  // One over the inclusive upper bound stays pending.
  assert.deepEqual(range.evaluate([{}, {}, {}, {}, {}, {}]), {
    done: false,
    reason: "count is 6, expected 3..=5",
  });
  // The zero-bottomed range holds the empty set (absence is an answer).
  const zeroed = new CountTracker({ kind: "between", min: 0, max: 2 });
  assert.deepEqual(zeroed.evaluate([]), { done: true, found: 0 });
});

test("the state word the typed timeout names is stable", () => {
  assert.equal(HAVE_COUNT_STATE, "have count");
});
