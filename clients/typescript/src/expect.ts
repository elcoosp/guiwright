/**
 * The client-side content- and identity-assertion evaluator (M54 — the
 * kit's `expect` vocabulary's content half over any door, the Rust seam's
 * `crates/guiwright-proto/src/expect.rs` mirrored row-for-row, the m45
 * one-milestone-per-surface pattern; the m62 identity kinds mirrored on
 * the TS client in M63). The vocabulary and the row-set evaluation live
 * here — a pure, transport-blind core — while the wire composition
 * (subscribe → describe-all re-reads → wake on the §7 event channel)
 * rides `GwpClient.expectText()` / `GwpClient.expectValue()` and the
 * identity rows `GwpClient.expectRole()` / `GwpClient.expectName()`.
 *
 * Four boundaries keep this honest (the Rust seam's rules, mirrored):
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
 *   wrong datum. The identity kinds (m62) read the row's structural
 *   facts instead: the role compared exactly, the name label-only. The
 *   assertion pins the set's first row (document order — the same pin
 *   the m42 wait speaks); later rows never answer.
 * - **Failures name names** (§8.3): the typed timeout carries the
 *   locator render, the budget and the last evaluation — expected and
 *   found. (The evaluator mirrors `test/expect.test.ts`'s pinned cases;
 *   the Rust tracker's cases are pinned in
 *   `crates/guiwright-proto/tests/expect_tracker.rs`.)
 */

import type {
  CheckedState,
  DescribedIdentityNode,
  DescribedIdentityStateNode,
  DescribedNode,
  TextMatchV1,
} from "./schema.ts";

/** The content- and identity-assertion vocabulary (the kit's
 * `to_have_text` / `to_have_value` rows, plus the M38 identity pair the
 * m62 seam rows carry: `to_have_role` / `to_have_accessible_name`) — a
 * client-side vocabulary, never serialized (the Rust `ExpectKind`
 * rule). The role kind's payload rides beside the tracker (a structural
 * fact takes no matcher); the other three carry a `TextMatchV1`. */
export type ExpectKind = "text" | "value" | "role" | "name" | "description";

/** The static state word the typed timeout names (the Rust
 * `ExpectKind::state()` wording — the kit's stuck-on convention carried
 * over the wire). */
export function expectState(kind: ExpectKind): string {
  if (kind === "text") return "have text";
  if (kind === "value") return "have value";
  if (kind === "description") return "have description";
  return kind === "role" ? "have role" : "have name";
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
 * `expectValue()` and the m63 identity rows `expectRole()` /
 * `expectName()`. Feed it fresh describe-all rows after every wake and
 * it answers the assertion; the evaluator carries no cross-wake state
 * (content and identity have no `stable`-like frame discipline — every
 * wake re-decides the whole read). */
export class ExpectTracker {
  private readonly kind: ExpectKind;
  private readonly expected: string | TextMatchV1;

  /** A tracker for one assertion, in the kind and expected fact it
   * asserts: the role kind carries the exact AccessKit debug name
   * (`Button`, `TextInput` — the vocabulary the role locators speak),
   * the other kinds a Playwright `TextMatchV1`. */
  constructor(kind: "role", expected: string);
  constructor(kind: Exclude<ExpectKind, "role">, expected: TextMatchV1);
  constructor(kind: ExpectKind, expected: string | TextMatchV1) {
    this.kind = kind;
    this.expected = expected;
  }

  /** Evaluate one row set: the set's head answers by the kind's read
   * (the m54 rule — document order, first row); an empty set stays
   * pending (waitable — nodes stream in, Playwright parity). The role
   * compares exactly (a structural fact — no matcher, no fuzz, the m62
   * rule); the name reads the label-only fact and a node with no
   * accessible name stays pending, the reason naming which fact is
   * missing. */
  evaluate(
    rows: (DescribedNode & { description?: string | null })[],
  ): ExpectEval {
    const head = rows[0];
    if (head === undefined) {
      return { done: false, reason: "no match yet (the set is empty)" };
    }
    // The identity rows (m62 — the m38 vocabulary over the door): the
    // role compares exactly, the name reads the label-only fact — the
    // value never answers a name assertion (the fact the text row
    // cannot pin).
    if (this.kind === "role") {
      const wanted = this.expected as string;
      if (head.role === wanted) {
        return { done: true, row: head };
      }
      return {
        done: false,
        reason: `role is ${JSON.stringify(head.role)}, expected ${JSON.stringify(wanted)}`,
      };
    }
    if (this.kind === "description") {
      // The m64/m65 kind: the description fact rides the identity rows
      // (the appended `describeAllIdentity()` read) — the description
      // only read, the name and the value never answer, and a missing
      // description stays pending naming the missing fact (the m38
      // rule: descriptions arrive with the node's restatement).
      const wanted = this.expected as TextMatchV1;
      const description = (head as DescribedIdentityNode).description ?? null;
      if (description === null) {
        return {
          done: false,
          reason: "the set's head carries no accessible description",
        };
      }
      if (textMatches(wanted, description)) {
        return { done: true, row: head };
      }
      return {
        done: false,
        reason: `description is ${JSON.stringify(description)}, expected ${renderMatch(wanted)}`,
      };
    }
    if (this.kind === "name") {
      const wanted = this.expected as TextMatchV1;
      if (head.name === null) {
        return {
          done: false,
          reason: "the set's head carries no accessible name",
        };
      }
      if (textMatches(wanted, head.name)) {
        return { done: true, row: head };
      }
      return {
        done: false,
        reason: `name is ${JSON.stringify(head.name)}, expected ${renderMatch(wanted)}`,
      };
    }
    // The content pair (m54): the m46 derivation orders — the text kind
    // reads the m29 order (name first, value fallback), the value kind
    // the m31 flip.
    const found =
      this.kind === "text" ? textContent(head) : valueContent(head);
    const wanted = this.expected as TextMatchV1;
    if (textMatches(wanted, found)) {
      return { done: true, row: head };
    }
    return {
      done: false,
      reason: `${this.kind} is ${JSON.stringify(found)}, expected ${renderMatch(wanted)}`,
    };
  }
}

/** The state-assertion vocabulary (m75 — the kit's `to_be_checked` /
 * `to_be_enabled` / `to_be_focused` rows over any door, the Rust seam's
 * `StateKind` mirrored, the m73 record's named standing gap closed on
 * the other-language client): a client-side vocabulary, never
 * serialized — the assertion composes the m54 wake loop with the m70
 * set-width state read (`describeAllIdentityState()` re-reads)
 * byte-for-byte, the `ExpectKind` rule. */
export type StateKind = "checked" | "enabled" | "focused";

/** The static state word the typed timeout names (the Rust
 * `StateKind::state()` wording — the kit's `to_be_*` shape carried over
 * the wire). */
export function stateWord(kind: StateKind): string {
  if (kind === "checked") return "be checked";
  if (kind === "enabled") return "be enabled";
  return "be focused";
}

/** One evaluation step's answer for the state kinds: either the fact
 * holds — carrying the m68 row that satisfied it (a directly usable
 * `action()` node, the dump-row rule) — or the reason it is still
 * pending, which becomes the timeout error's last-evaluation line. */
export type StateEval =
  | { done: true; row: DescribedIdentityStateNode }
  | { done: false; reason: string };

/** The row-set evaluator behind `GwpClient.expectChecked()` /
 * `expectEnabled()` / `expectFocused()` (the Rust `StateTracker`
 * mirrored row-for-row, the m63 evaluator shape at state width). Feed
 * it fresh describe-all-identity-state rows after every wake and it
 * answers the assertion; the evaluator carries no cross-wake state
 * (the state facts have no `stable`-like frame discipline — every wake
 * re-decides the whole read). */
export class StateTracker {
  private readonly kind: StateKind;
  private readonly wanted: CheckedState | boolean | undefined;

  /** A tracker for one assertion, in the kind and expected fact it
   * asserts: the checked kind carries the m68 four-way word (never a
   * boolean — a mixed checkbox is not an unchecked one), the enabled
   * kind the boolean the row carries, the focused kind nothing (the
   * fact itself is the demand). */
  constructor(kind: "checked", wanted: CheckedState);
  constructor(kind: "enabled", wanted: boolean);
  constructor(kind: "focused");
  constructor(kind: StateKind, wanted?: CheckedState | boolean) {
    this.kind = kind;
    this.wanted = wanted;
  }

  /** Evaluate one row set: the set's head answers by the kind's read
   * (the m54 rule — document order, first row); an empty set stays
   * pending (waitable — nodes stream in, Playwright parity). The
   * checked word is demanded exactly (the m73 tri-state rule), the
   * enabled boolean answers both ways, and the focused fact waits for
   * the focus to arrive. */
  evaluate(rows: DescribedIdentityStateNode[]): StateEval {
    const head = rows[0];
    if (head === undefined) {
      return { done: false, reason: "no match yet (the set is empty)" };
    }
    if (this.kind === "checked") {
      const wanted = this.wanted as CheckedState;
      if (head.checked === wanted) {
        return { done: true, row: head };
      }
      return {
        done: false,
        reason: `checked is ${JSON.stringify(head.checked)}, expected ${JSON.stringify(wanted)}`,
      };
    }
    if (this.kind === "enabled") {
      const wanted = this.wanted as boolean;
      if (head.enabled === wanted) {
        return { done: true, row: head };
      }
      return {
        done: false,
        reason: `enabled is ${head.enabled}, expected ${wanted}`,
      };
    }
    if (head.focused) {
      return { done: true, row: head };
    }
    return {
      done: false,
      reason: `focused is ${head.focused}, expected true`,
    };
  }
}
