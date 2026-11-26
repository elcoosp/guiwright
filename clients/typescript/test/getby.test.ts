/**
 * The TS `getBy*` locator factories (the m95 record's named TS pass):
 * the m95 honest list names the remaining half of the TS RegExp
 * acceptance pass — "the TS surface's `RegExp` acceptance —
 * `filter({ hasText: /re/ })`, the `getBy*` factories — is its own
 * pass (the m45/m60/m63 one-milestone-per-surface pattern)". The
 * `filter({ hasText: /re/ })` half shipped in the m95 stream; this file
 * pins the factory half: `getByRole` / `getByText` / `getByLabel` /
 * `getByValue` / `getByTestId` mirror the Rust kit's `get_by_*` sugars
 * (`crates/guiwright-kit/src/app.rs`), composing the wire's existing
 * rows client-side — the string forms ride the byte-pinned M0 rows
 * (the fuzzy `TextMatch` default, `exact` riding the match struct),
 * a `RegExp` rides the m95 regex rows through the same
 * `toRegexPattern` fold the filter helper speaks. No wire change: the
 * bytes pin against the Rust golden vectors (the M30/M89/M90/M91/M95
 * pin family) — the two clients fail the build together on drift.
 */

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

import { encodeRequest } from "../src/codec.ts";
import { encodeFrame } from "../src/framing.ts";
import { filter } from "../src/filter.ts";
import { getByRole, getByText, getByLabel, getByValue, getByTestId } from "../src/getby.ts";

const here = dirname(fileURLToPath(import.meta.url));
const golden = JSON.parse(readFileSync(join(here, "golden.json"), "utf8"));
const vectorByName = new Map(
  (golden.requests as { name: string; frame_hex: string }[]).map((v) => [v.name, v]),
);
const hexToBytes = (hex: string): Uint8Array => {
  const out = new Uint8Array(hex.length / 2);
  for (let i = 0; i < out.length; i += 1) out[i] = parseInt(hex.slice(i * 2, i * 2 + 2), 16);
  return out;
};

test("the factories compose the kit-mirrored string rows", () => {
  // The kit's sugar shape (app.rs get_by_role/get_by_label/get_by_text/
  // get_by_test_id/get_by_value): one selector row each, the fuzzy
  // TextMatch default on every text probe (substring,
  // case-insensitive).
  assert.deepEqual(getByRole("Button", { name: "Save" }), {
    kind: "role",
    role: "Button",
    name: { text: "Save", exact: false, ignore_case: true },
  });
  assert.deepEqual(getByRole("Button"), {
    kind: "role",
    role: "Button",
    name: null,
  });
  assert.deepEqual(getByLabel("Welcome, Ada"), {
    kind: "label",
    match: { text: "Welcome, Ada", exact: false, ignore_case: true },
  });
  assert.deepEqual(getByText("Install"), {
    kind: "text",
    match: { text: "Install", exact: false, ignore_case: true },
  });
  assert.deepEqual(getByValue("ada@example.dev"), {
    kind: "value",
    match: { text: "ada@example.dev", exact: false, ignore_case: true },
  });
  assert.deepEqual(getByTestId("btn-save"), {
    kind: "test_id",
    id: "btn-save",
  });
});

test("getByRole's exact option rides the exact TextMatchV1 (Playwright parity)", () => {
  // Playwright's `getByRole(role, { name, exact })`: exact means
  // case-sensitive whole-string — the Rust `TextMatch::exact` words
  // (the exact flag anchors, `ignore_case` drops).
  assert.deepEqual(getByRole("Button", { name: "Save", exact: true }), {
    kind: "role",
    role: "Button",
    name: { text: "Save", exact: true, ignore_case: false },
  });
  // The fuzzy default beside it, unchanged.
  assert.deepEqual(getByRole("Button", { name: "Save", exact: false }), {
    kind: "role",
    role: "Button",
    name: { text: "Save", exact: false, ignore_case: true },
  });
  // A full TextMatchV1 rides verbatim — the caller's own words (the
  // m92 filter() convention); `exact` never reshapes it.
  assert.deepEqual(
    getByRole("Button", { name: { text: "Save", exact: true, ignore_case: true } }),
    {
      kind: "role",
      role: "Button",
      name: { text: "Save", exact: true, ignore_case: true },
    },
  );
});

test("a RegExp rides the m95 regex rows through the shared fold", () => {
  // The flags fold inline (the wire rows carry the pattern only — one
  // shape per language), the refusal names the JS-only flags.
  assert.deepEqual(getByText(/save|cancel/i), {
    kind: "text_regex",
    pattern: "(?i)save|cancel",
  });
  assert.deepEqual(getByLabel(/^Welcome, Ada$/), {
    kind: "label_regex",
    pattern: "^Welcome, Ada$",
  });
  assert.deepEqual(getByValue(/^\S+@\S+\.\S+$/), {
    kind: "value_regex",
    pattern: "^\\S+@\\S+\\.\\S+$",
  });
  assert.deepEqual(getByRole("Button", { name: /^(Save|Cancel)$/ }), {
    kind: "role_name_regex",
    role: "Button",
    pattern: "^(Save|Cancel)$",
  });
  // Playwright's own rule: `exact` is a string option — a RegExp
  // matches as-is, the option never reshapes the pattern.
  assert.deepEqual(getByRole("Button", { name: /^Save$/, exact: true }), {
    kind: "role_name_regex",
    role: "Button",
    pattern: "^Save$",
  });
  // The JS-only flags refuse through the shared fold's typed error.
  assert.throws(() => getByText(/save/g), /flags \[g\] have no wire reading/);
});

test("the factories feed the existing composition helpers unchanged", () => {
  // A factory row is an ordinary Selector row: filter() narrows it,
  // the composition the kit's locator(child) sugar mirrors.
  const composed = filter(getByRole("Button"), { hasText: "Save", nth: "first" });
  assert.equal((composed as any).source.kind, "role");
  assert.equal((composed as any).source.name, null);
  assert.equal((composed as any).has_text.text, "Save");
  const narrowed = filter(getByRole("Button"), { hasText: /Sav/ });
  assert.equal((narrowed as any).source.kind, "has_text_regex");
});

test("the factory rows encode byte-identical to the Rust wire", () => {
  const builds: Record<string, () => ReturnType<typeof encodeRequest>> = {
    resolve_role_named: () => ({
      kind: "resolve",
      selector: getByRole("Button", { name: "Save" }),
      timeout_ms: 5000,
    }),
    resolve_role_unnamed: () => ({
      kind: "resolve",
      selector: getByRole("Button"),
      timeout_ms: 0,
    }),
    resolve_role_named_exact: () => ({
      kind: "resolve",
      selector: getByRole("Button", { name: "Save", exact: true }),
      timeout_ms: 5000,
    }),
    resolve_text: () => ({
      kind: "resolve",
      selector: getByText("Save"),
      timeout_ms: 5000,
    }),
    resolve_label: () => ({
      kind: "resolve",
      selector: getByLabel("Welcome, Ada"),
      timeout_ms: 5000,
    }),
    resolve_value: () => ({
      kind: "resolve",
      selector: getByValue("ada@example.dev"),
      timeout_ms: 5000,
    }),
    resolve_test_id: () => ({
      kind: "resolve",
      selector: getByTestId("btn-save"),
      timeout_ms: 0,
    }),
    resolve_text_regex: () => ({
      kind: "resolve",
      selector: getByText(/save|cancel/i),
      timeout_ms: 5000,
    }),
    resolve_label_regex: () => ({
      kind: "resolve",
      selector: getByLabel(/^Wel.* Ada$/),
      timeout_ms: 0,
    }),
    resolve_value_regex: () => ({
      kind: "resolve",
      selector: getByValue(/^\S+@\S+\.\S+$/),
      timeout_ms: 0,
    }),
    resolve_role_name_regex: () => ({
      kind: "resolve",
      selector: getByRole("Button", { name: /^(Save|Cancel)$/ }),
      timeout_ms: 5000,
    }),
  };
  for (const [name, build] of Object.entries(builds)) {
    const vector = vectorByName.get(name);
    assert.ok(vector, `golden vector for ${name} exists`);
    const built = encodeFrame(encodeRequest(build() as any));
    const expected = hexToBytes(vector.frame_hex);
    assert.deepEqual(
      Array.from(built),
      Array.from(expected),
      `${name}: the factory's row must byte-match the Rust golden`,
    );
  }
});
