/**
 * The TS `getBy*` locator factories (the m95 record's named TS pass —
 * the one-milestone-per-surface pattern's factory half): the five
 * Playwright-shaped factories the Rust kit's `get_by_*` sugars mirror
 * (`crates/guiwright-kit/src/app.rs` — `get_by_role` /
 * `get_by_label` / `get_by_text` / `get_by_test_id` /
 * `get_by_value`), composing the wire's existing `Selector` rows
 * client-side for the typed methods (`resolve` / `resolveAll` /
 * `describeAll*` / the `action` aim-and-act split). No wire change, no
 * locator object — the client stays request-row shaped (the m92
 * record's line); a factory returns the same `Selector` row a
 * hand-built one carries, so every composition helper
 * (`filter()`, the `all` chains) takes it unchanged.
 *
 * The text acceptance mirrors the kit's `impl Into<TextMatch>`: a bare
 * string rides the fuzzy default (substring, case-insensitive — the
 * Rust `TextMatch::new` words) on the byte-pinned string rows, a full
 * `TextMatchV1` rides verbatim (the caller's own words, the m92
 * filter() convention), and a `RegExp` rides the m95 regex rows
 * through the shared `toRegexPattern` fold (the pattern's own inline
 * flags decide case — the wire regex rows carry the pattern only, one
 * shape per language). `getByRole`'s `exact` option is Playwright
 * parity for the string form: `exact: true` rides the Rust
 * `TextMatch::exact` words (case-sensitive, whole-string). A `RegExp`
 * name matches as-is — Playwright's own rule, and the wire's (the
 * regex rows carry no exact flag; anchoring rides the pattern's own
 * `^…$`).
 */

import type { Selector, TextMatchV1 } from "./schema.ts";
import { roleNameRegex, textRegex, labelRegex, valueRegex } from "./filter.ts";

/** The text-probe acceptance (the kit's `impl Into<TextMatch>` in TS
 * words): a bare string becomes the fuzzy default, a full
 * `TextMatchV1` rides verbatim, a `RegExp` rides the m95 regex rows. */
export type TextProbe = string | TextMatchV1 | RegExp;

/** The fuzzy default for a string probe (the Rust `TextMatch::new`
 * words: substring, case-insensitive); a full `TextMatchV1` rides
 * verbatim; the `exact` option reshapes strings only (the Rust
 * `TextMatch::exact` words — case-sensitive, whole-string). */
function toMatchV1(match: TextProbe, exact = false): TextMatchV1 {
  if (typeof match === "string") {
    return exact
      ? { text: match, exact: true, ignore_case: false }
      : { text: match, exact: false, ignore_case: true };
  }
  return match;
}

/** The three string-probe rows share one composition: a `RegExp` rides
 * its appended regex row (the M64 rule at selector width), a string or
 * full `TextMatchV1` rides the byte-pinned string row. */
function probeRow(
  kind: "label" | "text" | "value",
  match: TextProbe,
  regexRow: (pattern: string | RegExp) => Selector,
): Selector {
  if (match instanceof RegExp) return regexRow(match);
  return { kind, match: toMatchV1(match) };
}

/** `getByRole(role, { name, exact })` — the kit's `get_by_role` sugar:
 * the role bucket, optionally narrowed by the accessible name. No
 * `name` composes the unnamed row (every widget of the role — the
 * kit's `role_locator` shape, strict mode at act time). The `exact`
 * option is Playwright parity for string names; a `RegExp` matches
 * as-is (its own inline flags decide case — the m95 wire rows carry
 * the pattern only). */
export function getByRole(
  role: string,
  opts: { name?: TextProbe; exact?: boolean } = {},
): Selector {
  const name = opts.name;
  if (name === undefined) {
    return { kind: "role", role, name: null };
  }
  if (name instanceof RegExp) {
    return roleNameRegex(role, name);
  }
  return { kind: "role", role, name: toMatchV1(name, opts.exact === true) };
}

/** `getByLabel(match)` — the kit's `get_by_label` sugar: the
 * accessible label probe (`getByLabel(/re/)` rides the regex row). */
export function getByLabel(match: TextProbe): Selector {
  return probeRow("label", match, labelRegex);
}

/** `getByText(match)` — the kit's `get_by_text` sugar: the node's own
 * label-or-value text probe (`getByText(/re/)` rides the regex row). */
export function getByText(match: TextProbe): Selector {
  return probeRow("text", match, textRegex);
}

/** `getByValue(match)` — the kit's `get_by_value` sugar: the form
 * value probe (`getByValue(/re/)` rides the regex row). */
export function getByValue(match: TextProbe): Selector {
  return probeRow("value", match, valueRegex);
}

/** `getByTestId(id)` — the kit's `get_by_test_id` sugar: the O(1)
 * hash probe (§8.1), the string riding verbatim. */
export function getByTestId(id: string): Selector {
  return { kind: "test_id", id };
}
