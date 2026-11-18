# M39 — The describe-all wire read (`DescribeAll` → `DescribedAll`)

**Status:** implemented. The §8.2 set read carrying facts — the m37
record's standing gap ("enumerating a set's facts would open one
`Describe` round trip per row — the m35 batch note, unchanged") closed
as one append-only wire domain: `DescribeAll { selector }` (request
discriminant 19) → `DescribedAll { rows }` (reply discriminant 22), the
`ResolveAll` walk's rows carrying the `Describe` identity facts, one
round trip instead of N. Session semantics are the m30 rule restated:
snapshot, non-strict, never waits, an empty set answers an empty list,
the connection's window scope narrows the walk. No session-shape change,
one budget row (`describe_all_wire`), golden-pinned on both languages.

---

## 1. Why now

M36 gave the REPL and the TS client a one-node facts read (`describe` /
`describe()`); M37 rendered its rows but recorded the cost shape of the
obvious follow-up honestly: a *set* facts read through the existing
vocabulary means N round trips, so the `all` rows stayed identity +
generation + rect. The read clients actually ask for at set width —
"what IS everything this selector matches" — is the
bulk-read family's missing member: `all_text_contents()` answers what
the set *says*, `describe_all()` answers what the set *is*. The
`ResolveAll` domain's payload (a bare selector) is exactly the request
shape, and the `DescribedV1` row is exactly the row shape — the
milestone is the walk plus the facts behind one discriminant pair.

## 2. Design: the resolve-all arm's shape, the describe row's facts

* **The payload reuses `ResolveAll`'s.** `DescribeAllV1 { selector }` —
  no new vocabulary to learn, the TS/REPL clients already speak it; the
  m30 discipline carries: no `timeout_ms` (the read never waits), an
  empty set answers an empty list, window scope narrows the walk
  (the server's set/read/clear rendezvous, byte-for-byte the
  `ResolveAll` arm's shape).
* **The rows reuse `DescribedV1`.** Each match's identity facts — role,
  accessible name, value, bounds — beside the generation the resolve
  minted. Every row stays directly usable as the `Action` node (the
  dump-row rule, M11): a facts read that answers "what should I act on"
  keeps the act-with-the-row loop intact.
* **Rows are freshly resolved, no stale-guard path.** The read
  resolves first and reads in the same pass, so the per-handle
  `StaleGeneration`/`UnknownNode` boundary `Describe` runs does not
  apply — the walk's rows cannot be stale by construction.
* **The REPL enrichment degrades gracefully.** `all <get-args…>` rides
  `DescribeAll` first; a driver that predates M39 falls back to the M35
  `ResolveAll` rows (the m37 hit-enrichment rule: the read is the
  answer, the facts ride one round trip).
* **Rendering is the established vocabulary.** The REPL `all` rows now
  print the describe row's rendering at set width
  (`Button "Save" (generation 0) rect=[…]`); the TS rows are the
  `describe()` objects at set width. Nothing new to learn.

## 3. Surface

- `crates/guiwright-proto/src/schema.rs` — the `DescribeAll`
  (`DescribeAllV1`) request and `DescribedAll` reply variants (the
  append-only discriminants 19 / 22), the wire-discipline docs.
- `crates/guiwright-proto/src/schemadoc.rs` — the published tables'
  two rows (the reviewed contract the clients vendor).
- `crates/guiwright-proto/src/client.rs` — the `ClientConn::describe_all`
  sugar (the `resolve_all`/`describe` pattern).
- `crates/guiwright-driver/src/session.rs` — `Session::describe_all`
  (pump → `resolve_all` walk → per-row facts, one pass).
- `crates/guiwright-driver/src/server.rs` — the `DescribeAll` arm (the
  `ResolveAll` arm's scope rendezvous, `DescribedV1` rows out).
- `crates/guiwright-kit/src/app.rs` — `Locator::describe_all()` (the
  bulk-read family row beside `all_text_contents()`/`all_values()`).
- `crates/guiwright-cli/src/repl.rs` — the `all` row's M39 enrichment +
  the documented fallback.
- `crates/guiwright-driver/examples/wire_describe_all.rs` — the
  consumer example (the set facts, the value row, the empty set).
- `clients/typescript/src/{schema,codec,client}.ts` — the
  `DescribeAll`/`DescribedAll` variants, the encode/decode arms, the
  `describeAll()` typed method.
- `clients/typescript/gwp-schema.json` + `test/golden.json` — the
  re-vendored contract and the blessed cross-language vectors.

## 4. Tests

- `crates/guiwright-proto/src/schema.rs` — the request/reply pins
  (19 / 22; the append-only guard chain extended).
- `crates/guiwright-proto/tests/golden_vectors.rs` — the
  `describe_all_buttons` request and `described_all_buttons` reply
  vectors (byte-pinned, blessed into the TS golden).
- `crates/guiwright-proto/tests/schema_golden.rs` — the variant-count
  probes (20 requests / 23 responses) + the blessed schema golden.
- `crates/guiwright-driver/tests/wire_describe.rs` —
  `describe_all_carries_the_set_facts_in_document_order` (four buttons,
  document order, hidden included, bounds restated; the empty set) +
  `the_describe_all_sugar_rides_the_trait` (the client seam) —
  end-to-end over the in-process door.
- `crates/guiwright-kit/tests/describe.rs` — the embedded half: the set
  facts in document order, the empty set, narrowing + the window pin.
- `crates/guiwright-cli/tests/repl_e2e.rs` — the `all` rows now pin the
  enriched rendering (`Button "Save" (generation 0) rect=…`), reads not
  resolve-stores intact.
- `clients/typescript/test/codec.test.ts` — the request encodes
  byte-identical to the Rust vector; the reply decodes to the golden
  shape; the vendored schema copy matches the variant tables.
- `clients/typescript/scripts/e2e.ts` — the cross-language leg: 4 fact
  rows in document order over real TCP, the empty set total, the
  byte-identical conversation through the M15 WS door.
- Red→green: the suites failed against the unfixed wire (unknown
  variant, missing sugar, the bare-handle `all` rows) and passed after
  the slice landed.

## 5. Budget

- `bench/budgets.toml` + the packaged crate copy — the
  `describe_all_wire` row (p50 900 µs / p95 4000 µs): one round trip on
  the four-button set — pump, the `resolve_cached_index` index walk,
  one consumer-tree node read per row, the row-set marshalled back.
- `crates/guiwright-bench/src/samples.rs` — the `describe_all_wire`
  sample; `crates/guiwright-bench/src/budgets.rs` — the registry entry.

## 6. Remaining gaps (honest list)

- No filtered describe-all: the request carries a bare selector (the
  `ResolveAll` payload); the kit's `.filter()`/`has_not` composition
  rides the selector vocabulary itself, so no second form exists yet —
  a suite that needs the filter options on the wire would extend
  `DescribeAllV1` append-only.
- No subscription push for set changes: the read is snapshot-shaped
  (the m30 rule); a client that wants arrivals subscribes and re-reads
  (the kit composes `wait_for` with the reads).
- The REPL fallback path (pre-M39 driver) is exercised by design review
  only — the suite always speaks to a same-version driver; the same
  honest note the m37 record carries for its enrichment.
