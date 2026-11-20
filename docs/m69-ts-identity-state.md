# M69 — The TypeScript client's single-node identity-state describe (`describeIdentityState()`)

**Status:** implemented. The m68 Rust rows mirrored on the other-language
client (the m45/m60/m63/m65/m67 one-milestone-per-surface pattern): the
`describeIdentityState(node, generation)` row over the appended
`describe_identity_state` domain — the M36 handle pair riding in, the
M68 identity-state row riding out — the cross-language e2e leg against
the real Rust server, and the consumer demo
(`npm run example:identity-state`). No wire change beyond the m68
domain the codec and variant tables already carry (the m68 record moved
them so this milestone's red step was the client method, not the
codec).

## 1. Surface

- `clients/typescript/src/schema.ts` — the variant tables' two rows
  (`DescribeIdentityState: 22` / `DescribedIdentityState: 25`), the
  `DescribedIdentityStateNode` interface (the M64 row's fields plus
  `focused` / `enabled` / `checked`) and the `CheckedState` word type
  (`"not_checkable" | "unchecked" | "checked" | "mixed"` — the m68
  four-way word, never `boolean | null`).
- `clients/typescript/src/codec.ts` — the encode arm (the M36 payload's
  exact shape, one variant byte off `describe_identity`) + the decode
  arm (variant 25 → the state row) + `decodeCheckedState` (the
  unit-variant index mapping, the postcard rule).
- `clients/typescript/src/client.ts` — the `describeIdentityState(node,
  generation)` row (the m68 `ClientConn::describe_identity_state`
  mirrored; the typed rejection rides `GwpRejected` / `codeName`, the
  Action boundary's honesty).
- `clients/typescript/test/codec.test.ts` — the manifest pins (the two
  golden names exist) + the request-encode fixture
  (`describe_identity_state_save_handle`, byte-identical to the Rust
  frame) + the vendored-schema parity rows (the frontier slices).
- `clients/typescript/test/client.test.ts` — the handle-width read
  against the golden-frame mock server (the m41/m67 harness shape, now
  serving the named variant): the blessed
  `described_identity_state_mixed_checkbox` frame replayed
  byte-for-byte, the state three asserted beside the identity four.
- `clients/typescript/scripts/e2e.ts` — the handle-width leg (the mixed
  tri-state, the disabled button's `enabled: false` +
  `not_checkable`, the ghost handle's typed `UnknownNode`) against the
  demo app, which serves the state fixture (the m68 fixture move's
  shape: the tree-snapshot pins move 9 → 12 with it, both doors).
- `clients/typescript/examples/describe_identity_state.ts` — the
  runnable demo (`npm run example:identity-state`).

## 2. Tests

- Red→green: the compile was the red step (the m45/m60/m63/m65/m67
  shape — `describeIdentityState` and the `described_identity_state`
  kind did not exist; the vendored-schema parity and the manifest pins
  failed against the un-served variant), the rows the green.
- `npm test` — 53 passing (the handle-width state pin added; the
  parity/frontier slices moved with the m68 tables).
- `npm run e2e` — the full conversation green including the m68/m69
  leg over TCP (and the M15 WS leg byte-identical): the mixed checkbox
  answers `checked: "mixed"`, the disabled `Confirm` button answers
  `enabled: false` + `checked: "not_checkable"`, an unfocused row
  answers `focused: false`, and the ghost handle rejects with the
  typed code the action on the same handle would meet.
- `npm run example:identity-state` — the demo conversation prints the
  tri-state trio beside the identity four, the disabled button's
  `enabled false` + `not_checkable`, an unfocused row's `focused
  false`, and the ghost handle's typed rejection.

## 3. Remaining gaps (honest list)

- No negation on the client: `not()` is the kit decorator's composition
  (M24); the client's honest negative is the typed timeout (the
  m54/m59/m63/m65/m67 rule, carried unchanged) — and for a total read,
  the typed rejection.
- No state facts at set width: `describeAllIdentity()`'s rows stay
  identity-row shaped (the m68 record's carried split); a state-fact
  set widening lands when a consumer asks — the same append-only move,
  its own domain.
