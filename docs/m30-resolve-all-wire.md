# M30 — The resolve-all wire domain (`ResolveAll` → `ResolvedAll`)

**Status:** implemented. One append-only request/reply pair on the V1
envelope, the typed `ClientConn::resolve_all` sugar, the TypeScript
`resolveAll()` client row with its golden vectors, one budget row — the
m22/m26/m29 records' named protocol half, closed with one golden pass.

---

## 1. Why now

Three records name the same open protocol question. M22 closed with
"`count` over GWP would ride a `Resolve` reply extension"; M26 carried
it forward ("over GWP the enumeration would ride the same `Resolve`
reply extension the count surface names"); M29 repeated it for the bulk
reads. Every deferred-half milestone since M20 shipped the embedded
surface first and left the wire half open — the one wire question the
parity stream never cashed. This milestone cashes it: the kit's set
read — `count()`'s non-strict walk, `all()`'s enumeration — becomes a
wire domain, so a TypeScript (or any other-language) client enumerates
a match set over any door instead of polling single resolves against
strict mode.

## 2. Design: one append-only pair, the set read's exact semantics

The load-bearing decision: **the domain speaks the set read's own
contract, not the strict resolve's.** `RequestV1::ResolveAll {
selector }` carries no `timeout_ms` — the embedded `resolve_all` never
waits (the snapshot-read rule M22 recorded), so a wire timeout field
would be a lie the server silently ignores. The reply,
`ResponseV1::ResolvedAll { matches: Vec<ResolvedMatchV1> }`, carries
one row per match with exactly the facts the single `Resolved` reply
already standardized (`node`, `generation`, `rect`) — so every client
that can decode one resolve can decode the set.

* **Append-only, byte-pinned.** The request appends at discriminant 16
  (after `Subscribe`), the reply at 19 (after `Subscribed`) — every
  pre-M30 client keeps its byte-exact encodings, enforced by the
  pinning tests and one golden-vector regeneration (the m29 record's
  "one golden pass"). `ResolvedMatchV1` is a new struct, not a reshape
  of `Resolved`; the single-resolve reply is untouched.
* **Rows are handles (the dump-row rule, M11).** Each row's `node` +
  `generation` is directly usable as an `Action` handle — enumerate,
  then act, no second resolve. The wire test proves it end-to-end.
* **Totality over the set.** An empty set answers with an empty list —
  never an error — and strict mode is never consulted: the ambiguity
  strict mode rejects single resolves with *is* the set read's data
  (the `count()` contract). Hidden nodes are included; document order
  is the ordering (the `all()`/`count()` semantics, unchanged over the
  wire).
* **The window scope rides the connection.** The `ResolveAll` arm runs
  the same scope discipline the `Resolve` arm runs — the connection's
  active target (§16 v0.4) narrows the walk, unknown handles fail with
  the typed `UnknownWindow` before any resolution.
* **The client sugar rides the trait.** `ClientConn::resolve_all` is a
  default method in terms of `Self::request` — the M6 window-methods
  pattern — so every shipped client and every downstream transport
  inherits the typed surface for free, and the TypeScript
  `resolveAll()` speaks the identical conversation (codec golden
  vector + cross-language e2e step).

No core, session, kit or REPL change: the session already exposed
`resolve_all`; the server arm is a scope + walk + marshal. The REPL row
stays out (see the honest list).

## 3. Surface

- `crates/guiwright-proto/src/schema.rs` — `RequestV1::ResolveAll`,
  `ResponseV1::ResolvedAll`, `ResolvedMatchV1` (the append-only pair +
  row struct, discriminant pins extended to 16/19).
- `crates/guiwright-proto/src/schemadoc.rs` — the published tables'
  rows 16/19 (the schema the other-language clients implement).
- `crates/guiwright-proto/src/client.rs` — `ClientConn::resolve_all`
  (the typed sugar, a default method like the M6 window methods).
- `crates/guiwright-driver/src/server.rs` — the `ResolveAll` arm
  (window scope + non-strict walk + row marshal).
- `crates/guiwright-viewer/src/live.rs` — the wire-misuse vocabulary
  gains `ResolvedAll` (the exhaustive match forces every consumer to
  move with the wire, on purpose).
- `clients/typescript` — `schema.ts` (consts + unions + the
  `ResolvedMatch` interface), `codec.ts` (encode + decode cases),
  `client.ts` (`resolveAll()`), the vendored `gwp-schema.json` and
  `test/golden.json` (the one golden pass).

## 4. Tests

- `crates/guiwright-proto/tests/golden_vectors.rs` — the
  `resolve_all_role` request vector (discriminant 16) and the
  `resolved_all` reply vector (two rows with rects), byte-pinned into
  `clients/typescript/test/golden.json` and re-decoded by the TS suite.
- `crates/guiwright-proto/tests/schema_golden.rs` — the exhaustive
  count fns move to 17/20 and the frontier spot-checks extend to
  `ResolveAll`/`ResolvedAll`; `schema-golden.json` blessed once.
- `crates/guiwright-driver/tests/wire_bulk_read.rs` — end-to-end on the
  in-process door: the four-button enumeration in document order (node
  ids 2, 3, 6, 7 — hidden included), the rect rule (bounded rows carry
  their rect, the hidden row carries `None`), the empty set answering
  `[]`, strict mode never consulted, the window-scoped set read on the
  two-window fixture (exactly Preview's Save), an enumerated row used
  directly as an `Action` handle, and the set read riding the generic
  `ClientConn` seam.
- `clients/typescript/test/codec.test.ts` — the `resolve_all_role`
  build entry encodes byte-identical to the Rust wire; the
  `resolved_all` vector decodes to the expected shape; the vendored
  schema copy's frontier slices extend.
- `clients/typescript/scripts/e2e.ts` — the cross-language e2e
  enumerates the demo server's four buttons over TCP before the action
  step (`✓ resolve-all: 4 matches in document order`).
- `bench/budgets.toml` + `crates/guiwright-bench/src/budgets.toml` —
  the `resolve_all_wire` row (sample: one `ResolveAll` over the
  in-process door — the non-strict walk plus four marshalled rows),
  enforced by the CI guard.

## 5. Remaining gaps (honest list)

- No REPL row — `resolve-all` is a client-surface command, and the REPL
  speaks command-shaped reads (`get`, `count` does not exist either);
  a REPL row lands with the REPL's own read-vocabulary pass, not as a
  rider on this domain.
- No kit-side change — the kit is embedded and already owns the
  client-side walk; a remote kit session would ride the wire domain
  exactly as the TS client does.
- The reply carries identities + rects, not contents: bulk *content*
  reads over the wire stay with the tree dump (`SnapshotNodeV1` rows
  already carry name + value) until a suite asks for a combined
  read — the m29 record's shape question, unchanged here.
- Comparator/count-constraint widening (`CountMatch` over the wire)
  stays out of scope — the set read returns the whole set, so every
  comparison stays client-side where it already lives.
