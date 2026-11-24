/**
 * The typed filter composition (the m34 record's remaining-gaps list:
 * *"other-language clients keep composing filters through the raw
 * selector rows — a typed `filter()` helper stays with the TS client's
 * own pass"*): `filter(source, opts)` builds the `SelectorV1::Filter`
 * row the wire already spoke — the string form (`hasText`, the Rust
 * `TextMatch` default: substring, case-insensitive), the inverted
 * selector form (`hasNot`) and the document-order pick (`nth`) — and
 * the m91 `has` option speaks the appended positive-filter variant
 * (`SelectorV1::Has`), no raw selector rows to hand-assemble.
 *
 * Composition order mirrors the Rust kit's locator sugar: `has` wraps
 * the source in its own row first (the widening is its own variant — the
 * wire's `Filter` payload never grew, the M64 rule at selector width),
 * then the `filter` row narrows the wrapped set. The result composes
 * with every other selector row (`all` chains, nested `filter` calls —
 * `filter(filter(source, …), …)` nests the narrowing the resolver
 * walks).
 */

import type { Nth, Selector, TextMatchV1 } from "./schema.ts";

/** The filter options (`Locator.filter`'s keys — Playwright's
 * `filter({ hasText, hasNot, has })` plus the kit's document-order
 * pick). */
export type FilterOpts = {
  /** The string form: a bare string rides the Rust `TextMatch` default
   * (substring, case-insensitive); a full `TextMatchV1` passes as-is. */
  hasText?: string | TextMatchV1;
  /** The inverted selector form: drop matches whose subtree (the node
   * included) fits it. */
  hasNot?: Selector;
  /** The selector-valued positive filter: keep matches whose subtree
   * (the node included) contains a node matching it — rides the
   * appended `SelectorV1::Has` row. */
  has?: Selector;
  /** The document-order pick: `"first"`, `"last"` or an index. */
  nth?: "first" | "last" | number;
};

/** A bare string becomes the fuzzy `TextMatchV1` (the Rust
 * `TextMatch::new` default — substring, case-insensitive); a full
 * `TextMatchV1` rides unchanged. */
function toTextMatch(match: string | TextMatchV1): TextMatchV1 {
  return typeof match === "string"
    ? { text: match, exact: false, ignore_case: true }
    : match;
}

function toNth(nth: NonNullable<FilterOpts["nth"]>): Nth {
  if (nth === "first") return { kind: "first" };
  if (nth === "last") return { kind: "last" };
  return { kind: "index", index: nth };
}

/** Compose the filter rows over `source` (see the module docs — `has`
 * rides its own appended row, the rest build one `filter` row). With no
 * options the source returns unchanged. */
export function filter(source: Selector, opts: FilterOpts = {}): Selector {
  let current = source;
  if (opts.has !== undefined) {
    current = { kind: "has", source: current, has: opts.has };
  }
  if (opts.hasText === undefined && opts.hasNot === undefined && opts.nth === undefined) {
    return current;
  }
  return {
    kind: "filter",
    source: current,
    has_text: opts.hasText !== undefined ? toTextMatch(opts.hasText) : null,
    has_not: opts.hasNot ?? null,
    nth: opts.nth !== undefined ? toNth(opts.nth) : null,
  };
}
