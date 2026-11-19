/**
 * The client-side count-assertion evaluator (m59/m60 — the kit's
 * `expect` vocabulary's count half over any door, the Rust seam's
 * `crates/guiwright-proto/src/count.rs` mirrored row-for-row, the m45
 * one-milestone-per-surface pattern). The vocabulary and the set
 * evaluation live here — a pure, transport-blind core — while the wire
 * composition (subscribe → resolve-all re-reads → wake on the §7 event
 * channel) rides `GwpClient.expectCount()` and its comparator rows.
 *
 * The boundaries keep this honest (the Rust seam's rules, mirrored):
 *
 * - **No wire change.** The comparison below is a client-side
 *   vocabulary (never serialized): the assertion composes the existing
 *   `subscribe`, `resolveAll()` and event domains byte-for-byte as
 *   M12/M30 blessed them. A driver that predates M30 rejects
 *   `resolveAll()` with the typed `Unsupported` and the assertion
 *   surfaces that rejection rather than degrading (the m41 fail-fast
 *   rule).
 * - **The set is the truth.** Evaluation sees exactly what the M30
 *   `resolveAll()` walk answers — the whole match set, strict mode
 *   never consulted (the m22 record's set read). The kit's count
 *   vocabulary composes this loop over `resolveAll()` instead of the
 *   m54 content pair's `describeAll()`: a count is a fact about the
 *   set, so the set read is the honest round trip. Absence is an
 *   answer: an empty set is found 0 — satisfying `count 0` and every
 *   less-than (Playwright parity), unlike the content reads'
 *   empty-set pending.
 * - **Failures name names** (§8.3): the typed timeout carries the
 *   locator render, the budget and the last evaluation — the demanded
 *   comparison and the count the last evaluation found. (The evaluator
 *   mirrors `test/count.test.ts`'s pinned cases; the Rust tracker's
 *   cases are pinned in the `count` module.)
 */

/** The count-assertion vocabulary (the kit's `to_have_count` family:
 * the M22 exact form + the M25 one-sided comparators) — a client-side
 * vocabulary, never serialized (the Rust `CountMatch` rule). */
export type CountCompare = {
  kind: "exact" | "greater_than" | "less_than";
  bound: number;
};

/** The static state word the typed timeout names (the Rust
 * `HAVE_COUNT_STATE` wording — the m54 `have text` / `have value`
 * shape, count family). */
export const HAVE_COUNT_STATE = "have count";

/** Whether `found` satisfies the comparison (the Rust
 * `CountMatch::holds` predicate, mirrored). */
function holds(compare: CountCompare, found: number): boolean {
  if (compare.kind === "exact") return found === compare.bound;
  if (compare.kind === "greater_than") return found > compare.bound;
  return found < compare.bound;
}

/** The demanded comparison's render for the timeout's "expected" side
 * (the core's `count 4` / `count > 3` / `count < 5` vocabulary, the
 * exact form the kit's headers print). */
function renderExpected(compare: CountCompare): string {
  if (compare.kind === "exact") return String(compare.bound);
  if (compare.kind === "greater_than") return `> ${compare.bound}`;
  return `< ${compare.bound}`;
}

/** One evaluation step's answer: either the comparison holds — carrying
 * the count that satisfied it (the fact the assertion speaks, the m57
 * summary shape) — or the reason it is still pending, which becomes the
 * timeout error's last-evaluation line. */
export type CountEval =
  | { done: true; found: number }
  | { done: false; reason: string };

/** The set evaluator behind `GwpClient.expectCount()` and its
 * comparator rows. Feed it fresh resolve-all sets after every wake and
 * it answers the assertion; the evaluator carries no cross-wake state
 * (a count has no `stable`-like frame discipline — every wake re-decides
 * the whole read, the m54 rule). */
export class CountTracker {
  private readonly comparison: CountCompare;

  /** A tracker for one assertion, in the comparison it demands. */
  constructor(comparison: CountCompare) {
    this.comparison = comparison;
  }

  /** Evaluate one match set: the whole set answers (the m22 record's
   * set read), an empty set is found 0 — an answer like any other,
   * satisfying `count 0` and every less-than (absence is an answer). */
  evaluate(matches: unknown[]): CountEval {
    const found = matches.length;
    if (holds(this.comparison, found)) {
      return { done: true, found };
    }
    return {
      done: false,
      reason: `count is ${found}, expected ${renderExpected(this.comparison)}`,
    };
  }
}
