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

test("the state word the typed timeout names is stable", () => {
  assert.equal(HAVE_COUNT_STATE, "have count");
});
