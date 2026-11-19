/**
 * The client-side content-assertion evaluator (M54 — the kit's `expect`
 * vocabulary's content half over any door, the Rust seam's
 * `crates/guiwright-proto/src/expect.rs` mirrored row-for-row, the m45
 * one-milestone-per-surface pattern). The vocabulary and the row-set
 * evaluation live here — a pure, transport-blind core — while the wire
 * composition (subscribe → describe-all re-reads → wake on the §7 event
 * channel) rides `GwpClient.expectText()` / `GwpClient.expectValue()`.
 *
 * Three boundaries keep this honest (the Rust seam's rules, mirrored):
 *
 * - **No wire change.** The kinds below are a client-side vocabulary
 *   (never serialized): the assertion composes the existing
 *   `subscribe`, `describeAll()` and event domains byte-for-byte as
 *   M12/M39 blessed them. A driver that predates M39 rejects
 *   `describeAll()` with the typed `Unsupported` and the assertion
 *   surfaces that rejection rather than degrading (the m41 fail-fast
 *   rule).
 * - **The rows are the truth.** Evaluation sees exactly what the M39
 *   `describeAll()` walk answers, through the M41 derivation orders:
 *   the `text` kind reads the m29 order (accessible name first, value
 *   fallback) and the `value` kind reads the m31 order (value first,
 *   name fallback) — the two reads can never silently agree on the
 *   wrong datum. The assertion pins the set's first row (document
 *   order — the same pin the m42 wait speaks); later rows never answer.
 * - **Failures name names** (§8.3): the typed timeout carries the
 *   locator render, the budget and the last evaluation — expected and
 *   found. (The evaluator mirrors `test/expect.test.ts`'s pinned cases;
 *   the Rust tracker's cases are pinned in
 *   `crates/guiwright-proto/tests/expect_tracker.rs`.)
 */

import type { DescribedNode, TextMatchV1 } from "./schema.ts";

/** The content-assertion vocabulary (the kit's `to_have_text` /
 * `to_have_value` rows) — a client-side vocabulary, never serialized
 * (the Rust `ExpectKind` rule). */
export type ExpectKind = "text" | "value";

/** The static state word the typed timeout names (the Rust
 * `ExpectKind::state()` wording). */
export function expectState(kind: ExpectKind): string {
  return kind === "text" ? "have text" : "have value";
}

/** The row's text content (the m29 order): the accessible name first,
 * the value as the fallback, `""` when neither fact rides the row. */
function textContent(row: DescribedNode): string {
  return row.name ?? row.value ?? "";
}

/** The row's value content (the m31 order): the value first, the
 * accessible name as the fallback — the form-dump flip. */
function valueContent(row: DescribedNode): string {
  return row.value ?? row.name ?? "";
}

/** Collapse runs of whitespace to single spaces and trim the ends
 * (Playwright's text normalization — the core `normalize()` rule). */
function normalize(text: string): string {
  return text.replace(/\s+/g, " ").trim();
}

/** The Playwright match rule (the core `TextMatch::matches`): both
 * sides whitespace-normalized; substring + case-insensitive unless
 * `exact`, which is full-string + case-sensitive (after
 * normalization). */
export function textMatches(m: TextMatchV1, candidate: string): boolean {
  const c = normalize(candidate);
  const needle = normalize(m.text);
  if (m.ignore_case) {
    const lc = c.toLowerCase();
    const ln = needle.toLowerCase();
    return m.exact ? lc === ln : lc.includes(ln);
  }
  return m.exact ? c === needle : c.includes(needle);
}

/** The wanted matcher's render (the core's check vocabulary shape: the
 * exact pattern quoted bare, the fuzzy one suffixed — the Rust
 * `render()` rule the timeout lines quote). */
function renderMatch(m: TextMatchV1): string {
  const quoted = JSON.stringify(m.text);
  return m.exact && !m.ignore_case ? quoted : `${quoted} (fuzzy)`;
}

/** One evaluation step's answer: either the assertion holds — carrying
 * the row that satisfied it (a directly usable `action()` node, the
 * dump-row rule) — or the reason it is still pending, which becomes the
 * timeout error's last-evaluation line. */
export type ExpectEval =
  | { done: true; row: DescribedNode }
  | { done: false; reason: string };

/** The row-set evaluator behind `GwpClient.expectText()` /
 * `expectValue()`. Feed it fresh describe-all rows after every wake and
 * it answers the assertion; the evaluator carries no cross-wake state
 * (content has no `stable`-like frame discipline — every wake re-decides
 * the whole read). */
export class ExpectTracker {
  private readonly kind: ExpectKind;
  private readonly expected: TextMatchV1;

  /** A tracker for one assertion, in the kind and matcher it asserts. */
  constructor(kind: ExpectKind, expected: TextMatchV1) {
    this.kind = kind;
    this.expected = expected;
  }

  /** Evaluate one row set: the set's head answers by the kind's source
   * order (the m46 derivations, reused verbatim — one boundary, stated
   * once); an empty set stays pending (waitable — nodes stream in,
   * Playwright parity). */
  evaluate(rows: DescribedNode[]): ExpectEval {
    const head = rows[0];
    if (head === undefined) {
      return { done: false, reason: "no match yet (the set is empty)" };
    }
    const found = this.kind === "text" ? textContent(head) : valueContent(head);
    if (textMatches(this.expected, found)) {
      return { done: true, row: head };
    }
    return {
      done: false,
      reason: `${this.kind} is ${JSON.stringify(found)}, expected ${renderMatch(this.expected)}`,
    };
  }
}
