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
 * The regex text probes (the m34 record's second standing note, cashed
 * — the m93/m94 honest lists' first item, the m95 record): `hasText`
 * accepts a `RegExp` and rides the appended `has_text_regex` row, and
 * the five row builders (`textRegex`, `labelRegex`, `valueRegex`,
 * `roleNameRegex`, `hasTextRegex`) compose the appended rows the Rust
 * kit's `get_by_*`/`has_text` sugars mirror — a `RegExp` folds its
 * flags into the pattern's own inline form (`i`/`m`/`s`; the wire rows
 * carry the pattern only, one shape per language), a string rides
 * verbatim. The JS `g`/`y`/`d`/`u` flags have no wire reading — the
 * typed refusal names them rather than silently dropping semantics.
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
   * (substring, case-insensitive); a full `TextMatchV1` passes as-is; a
   * `RegExp` rides the appended `has_text_regex` row (the m95 regex
   * family — the pattern source with the flags folded inline). */
  hasText?: string | TextMatchV1 | RegExp;
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

/** The regex probe's pattern source (the m95 family): a string rides
 * verbatim (the inline flags are the pattern's own words — `(?i)save`);
 * a `RegExp` folds its source with the flags that have a wire reading —
 * `i`/`m`/`s` become the pattern's inline `(?flags)` prefix. The JS
 * `g`/`y`/`d`/`u` flags have no wire reading (matching-mode or
 * Unicode-mode, not pattern semantics): the typed refusal names them —
 * never a silent semantic drop.
 *
 * Throws when the RegExp carries a flag the wire cannot speak. */
export function toRegexPattern(match: string | RegExp): string {
  if (typeof match === "string") return match;
  const silent = match.flags.match(/[gydu]/g);
  if (silent !== null) {
    throw new TypeError(
      `regex flags [${silent.join("")}] have no wire reading — ` +
        "drop them (g/y are matching mode, d/u are parser modes) or build " +
        "the row by hand with the pattern source",
    );
  }
  const carried = match.flags.match(/[ims]/g)?.join("") ?? "";
  return carried.length > 0 ? `(?${carried})${match.source}` : match.source;
}

/** The regex text probe (the m95 family): `getByText(/re/)`'s row —
 * the node's own label-or-value text against the pattern (partial by
 * default, case via the pattern's inline flags). */
export function textRegex(pattern: string | RegExp): Selector {
  return { kind: "text_regex", pattern: toRegexPattern(pattern) };
}

/** The regex label probe — `getByLabel(/re/)`'s row. */
export function labelRegex(pattern: string | RegExp): Selector {
  return { kind: "label_regex", pattern: toRegexPattern(pattern) };
}

/** The regex value probe — `getByValue(/re/)`'s row. */
export function valueRegex(pattern: string | RegExp): Selector {
  return { kind: "value_regex", pattern: toRegexPattern(pattern) };
}

/** The regex role-name probe — `getByRole(role, { name: /re/ })`'s
 * row: the role bucket narrowed by the pattern. */
export function roleNameRegex(role: string, pattern: string | RegExp): Selector {
  return { kind: "role_name_regex", role, pattern: toRegexPattern(pattern) };
}

/** The regex `has_text` filter — `filter({ hasText: /re/ })`'s own
 * row (the appended variant, the M64 rule at selector width): the
 * source's matches whose text — the node included, the subtree walk —
 * fits the pattern. */
export function hasTextRegex(source: Selector, pattern: string | RegExp): Selector {
  return { kind: "has_text_regex", source, pattern: toRegexPattern(pattern) };
}

function toNth(nth: NonNullable<FilterOpts["nth"]>): Nth {
  if (nth === "first") return { kind: "first" };
  if (nth === "last") return { kind: "last" };
  return { kind: "index", index: nth };
}

/** Compose the filter rows over `source` (see the module docs — `has`
 * rides its own appended row, a `RegExp` `hasText` rides the appended
 * regex row, the rest build one `filter` row). With no options the
 * source returns unchanged. */
export function filter(source: Selector, opts: FilterOpts = {}): Selector {
  let current = source;
  if (opts.has !== undefined) {
    current = { kind: "has", source: current, has: opts.has };
  }
  if (opts.hasText !== undefined && opts.hasText instanceof RegExp) {
    // The RegExp form rides its own appended row (the m95 family, the
    // M64 rule at selector width — the `Filter` payload never grew);
    // the `nth`/`hasNot` beside it compose the regex row as their
    // source, the fold's own composition.
    current = hasTextRegex(current, opts.hasText);
    return {
      kind: "filter",
      source: current,
      has_text: null,
      has_not: opts.hasNot ?? null,
      nth: opts.nth !== undefined ? toNth(opts.nth) : null,
    };
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
