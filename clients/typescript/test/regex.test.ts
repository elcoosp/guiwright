/**
 * The TypeScript regex text rows (the m34 record's second standing
 * note, cashed — the m93/m94 honest lists' first item, the m95
 * record's TS half): the five appended `SelectorV1` rows composed
 * client-side by the typed builders (`textRegex`, `labelRegex`,
 * `valueRegex`, `roleNameRegex`, `hasTextRegex`), the `RegExp`
 * acceptance folding the flags that have a wire reading
 * (`i`/`m`/`s` → the pattern's own inline form) and refusing the ones
 * that do not (`g`/`y`/`d`/`u` — the typed refusal, never a silent
 * semantic drop), and `filter({ hasText: /re/ })` riding the regex row
 * beside the string `Filter` form. The bytes pin against the Rust
 * golden vectors (the M30/M89/M90/M91 pin family) — the two clients
 * fail the build together on drift.
 */

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

import { encodeRequest } from "../src/codec.ts";
import { encodeFrame } from "../src/framing.ts";
import {
  filter,
  textRegex,
  labelRegex,
  valueRegex,
  roleNameRegex,
  hasTextRegex,
  toRegexPattern,
} from "../src/filter.ts";

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

test("the row builders compose the appended SelectorV1 rows", () => {
  assert.deepEqual(textRegex("(?i)save|cancel"), {
    kind: "text_regex",
    pattern: "(?i)save|cancel",
  });
  assert.deepEqual(labelRegex("^Wel.* Ada$"), {
    kind: "label_regex",
    pattern: "^Wel.* Ada$",
  });
  assert.deepEqual(valueRegex("^\\S+@\\S+\\.\\S+$"), {
    kind: "value_regex",
    pattern: "^\\S+@\\S+\\.\\S+$",
  });
  assert.deepEqual(roleNameRegex("Button", "^(Save|Cancel)$"), {
    kind: "role_name_regex",
    role: "Button",
    pattern: "^(Save|Cancel)$",
  });
  assert.deepEqual(
    hasTextRegex({ kind: "role", role: "Button", name: null }, "^Save$"),
    {
      kind: "has_text_regex",
      source: { kind: "role", role: "Button", name: null },
      pattern: "^Save$",
    },
  );
});

test("a RegExp folds the wire-readable flags inline and refuses the rest", () => {
  // The case flag folds: /save|cancel/i rides as (?i)save|cancel — the
  // pattern's own words (one wire shape per language).
  assert.equal(toRegexPattern(/save|cancel/i), "(?i)save|cancel");
  assert.equal(toRegexPattern(/^save$/), "^save$", "no flags — the source verbatim");
  assert.equal(toRegexPattern(/a.b/ms), "(?ms)a.b", "m and s carry their Rust inline forms");
  // The matching-mode flags have no wire reading — the typed refusal.
  assert.throws(() => toRegexPattern(/save/g), /flags \[g\] have no wire reading/);
  assert.throws(() => toRegexPattern(/save/gydu), /flags \[dguy\] have no wire reading/);
  // The builders compose RegExps through the same fold.
  assert.deepEqual(textRegex(/save/i), { kind: "text_regex", pattern: "(?i)save" });
});

test("filter({ hasText: /re/ }) rides the appended regex row (the M64 rule)", () => {
  // The RegExp form: the regex row wraps the source, the string-Filter
  // row carries no has_text — the fold's own composition, the
  // byte-pinned Filter payload never reshaped.
  assert.deepEqual(
    filter({ kind: "role", role: "Button", name: null }, { hasText: /^Sav/ }),
    {
      kind: "filter",
      source: {
        kind: "has_text_regex",
        source: { kind: "role", role: "Button", name: null },
        pattern: "^Sav",
      },
      has_text: null,
      has_not: null,
      nth: null,
    },
  );
  // The string form beside it: the fuzzy default, unchanged bytes.
  assert.equal(
    (filter({ kind: "role", role: "Button", name: null }, { hasText: "Save" }) as any)
      .has_text.text,
    "Save",
  );
  // The `has` option composes first, the regex row over it.
  const composed = filter(
    { kind: "role", role: "Group", name: null },
    { has: { kind: "role", role: "Button", name: null }, hasText: /cancel/i },
  );
  assert.equal((composed as any).source.kind, "has_text_regex");
  assert.equal((composed as any).source.source.kind, "has");
});

test("the regex rows encode byte-identical to the Rust wire", () => {
  const builds: Record<string, () => ReturnType<typeof encodeRequest>> = {
    resolve_text_regex: () => ({
      kind: "resolve",
      selector: textRegex(/save|cancel/i),
      timeout_ms: 5000,
    }),
    resolve_label_regex: () => ({
      kind: "resolve",
      selector: labelRegex("^Wel.* Ada$"),
      timeout_ms: 0,
    }),
    resolve_value_regex: () => ({
      kind: "resolve",
      selector: valueRegex("^\\S+@\\S+\\.\\S+$"),
      timeout_ms: 0,
    }),
    resolve_role_name_regex: () => ({
      kind: "resolve",
      selector: roleNameRegex("Button", /^(Save|Cancel)$/),
      timeout_ms: 5000,
    }),
    resolve_has_text_regex: () => ({
      kind: "resolve",
      selector: hasTextRegex({ kind: "role", role: "Button", name: null }, "^Save$"),
      timeout_ms: 0,
    }),
    resolve_has_text_regex_via_filter: () => ({
      kind: "resolve",
      selector: filter({ kind: "role", role: "Button", name: null }, { hasText: /^Save$/ }),
      timeout_ms: 0,
    }),
  };
  for (const [name, build] of Object.entries(builds)) {
    if (name.endsWith("_via_filter")) continue; // semantic twin, asserted below
    const vector = vectorByName.get(name);
    assert.ok(vector, `golden vector for ${name} exists`);
    const built = encodeFrame(encodeRequest(build()));
    const expected = hexToBytes(vector.frame_hex);
    assert.deepEqual(
      Array.from(built),
      Array.from(expected),
      `${name} must encode to the exact Rust bytes`,
    );
  }
});

test("filter's regex row encodes byte-identical to the hand-built composition", () => {
  // The semantic twin: the helper composes the regex row as the source
  // of the string-Filter row (the fold's own composition — the Rust
  // convert's shape); the hand-built form nests the same rows.
  const viaFilter = encodeFrame(
    encodeRequest({
      kind: "resolve",
      selector: filter({ kind: "role", role: "Button", name: null }, { hasText: /^Save$/ }),
      timeout_ms: 0,
    }),
  );
  const byHand = encodeFrame(
    encodeRequest({
      kind: "resolve",
      selector: {
        kind: "filter",
        source: {
          kind: "has_text_regex",
          source: { kind: "role", role: "Button", name: null },
          pattern: "^Save$",
        },
        has_text: null,
        has_not: null,
        nth: null,
      },
      timeout_ms: 0,
    }),
  );
  assert.deepEqual(Array.from(viaFilter), Array.from(byHand));
  // And the bare regex row (the builder's own shape) pins against the
  // golden vector it was blessed from.
  const bare = vectorByName.get("resolve_has_text_regex");
  assert.ok(bare);
  const viaBuilder = encodeFrame(
    encodeRequest({
      kind: "resolve",
      selector: hasTextRegex({ kind: "role", role: "Button", name: null }, "^Save$"),
      timeout_ms: 0,
    }),
  );
  assert.deepEqual(Array.from(viaBuilder), Array.from(hexToBytes(bare.frame_hex)));
});
