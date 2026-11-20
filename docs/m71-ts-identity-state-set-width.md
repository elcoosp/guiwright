# M71 — The TypeScript client's set-width identity-state describe (`describeAllIdentityState`)

**Status:** implemented. The m70 record's named standing gap, cashed:
the m70 Rust row mirrored on the other-language client (the
m45/m60/m63/m65/m67/m69 one-milestone-per-surface pattern). The m70
record moved the codec, the variant tables and the byte pins already —
so this record's red step was the client method, not the codec. No wire
change.

---

## 1. Surface

- `clients/typescript/src/schema.ts` — the `RequestV1.DescribeAllIdentityState: 23`
  and `ResponseV1.DescribedAllIdentityState: 26` table entries, the
  `describe_all_identity_state` request kind, the
  `described_all_identity_state` reply member (the m68
  `DescribedIdentityStateNode` per row).
- `clients/typescript/src/codec.ts` — the encode arm (the `describeAll`
  walk payload's exact shape: the frame differs from
  `describe_all_identity` only in the variant byte) and the decode arm
  (the m68 row decoder per match, document order — the m64 set-decode
  loop's shape).
- `clients/typescript/src/client.ts` — the `describeAllIdentityState(selector)`
  row (the m70 Rust `ClientConn::describe_all_identity_state` mirrored;
  fail-fast, no fallback — a driver older than m70 cannot decode the
  appended request at all, the append-only cost).
- `clients/typescript/test/golden.json` — the m70 frames the Rust bless
  wrote (`describe_all_identity_state_checkboxes` /
  `described_all_identity_state_checkboxes`); the TS side re-pins the
  vendor copy in `test/codec.test.ts` (the pin assertions, the builder
  map, the vendored-table frontier lists) and replays the blessed
  reply over a live mock server in `test/client.test.ts`.
- `clients/typescript/scripts/e2e.ts` — the cross-language leg against
  the real Rust demo server (the trio's tri-state per row in document
  order, the narrowed disabled walk, the honest empty list).
- `clients/typescript/examples/describe_identity_state_all.ts` — the
  consumer demo (`npm run example:identity-state-all`).
- `clients/typescript/package.json` — the demo's npm wiring.

## 2. Tests

- Red→green: the client method was the red step (the m45/m60/m63/m65/m67/m69
  seam-half shape — `describeAllIdentityState` did not exist), the
  codec pins the runtime red (the builder map missing the m70 vector),
  the rows the green.
- `npm test` — 54 passing (was 53): the codec pin
  (`describe_all_identity_state_checkboxes` /
  `described_all_identity_state_checkboxes` byte-identical to the Rust
  frames), the vendored-schema parity (the tables' frontier lists), the
  client replay pin (the blessed m70 reply decoded over a live mock
  server, the tri-state riding per row).
- The cross-language e2e leg against the real Rust server
  (`npm run e2e`): the checkbox trio's `checked` / `unchecked` /
  `mixed` in document order from one round trip, the narrowed walk
  answering the disabled button (`enabled: false` +
  `not_checkable`), the empty set's honest empty list.
- The demo conversation is runnable: `npm run example:identity-state-all`
  spawns the Rust demo app and prints the walk's tri-state per row, the
  disabled button's `enabled false`, and the empty set's honest empty
  list. The Rust-side mirror is `cargo run -p guiwright-driver --example
  wire_client_identity_state_all`.

## 3. Remaining gaps (honest list)

- No kit-side read over the state rows: the kit's `describe` /
  `describe_all` keep their `NodeDescription` surfaces; a kit-side
  `describe_state` / `describe_all_state` lands when a consumer asks
  (the m68/m70 records' kit note, carried unchanged).
- No negation on the seam: `.not()` is the kit decorator's composition
  (M24); the seam's honest negative is the typed timeout (the
  m54/m59/m64/m67/m68/m70 rule, carried unchanged).
- No budget row: the m54/m62/m64/m66/m68/m70 rule carried forward (the
  read rides the budgeted `describe_all_wire` conversation shape).
