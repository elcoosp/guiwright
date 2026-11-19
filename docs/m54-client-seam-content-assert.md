# M54 — The client seam's content assertions (`expect_text` / `expect_value`)

**Status:** implemented, both surfaces. Two default methods on the
[`ClientConn`] seam (`crates/guiwright-proto/src/client.rs`) plus the
client-side [`ExpectKind`] / [`ExpectTracker`] evaluator
(`crates/guiwright-proto/src/expect.rs`) — the kit's `expect` vocabulary's
*content* half over any door, the one-milestone-per-surface pattern
(m46→…→m53) carried to the §8 check vocabulary, the half the m42 wait's
state rows left open: a remote Rust suite could wait attached/visible/
hidden/stable over the door (m42) and read the set's content (m46), but
its assertions were hand-rolled `describe` reads with ad-hoc retries.
Now `expect_text(selector, expected, timeout)` re-reads the describe-all
walk on every §7 wake until the content matches or the budget expires —
the m29 source order (label first) — and `expect_value(…)` the m31 flip
(value first). The TypeScript client carries the same rows on its own
milestone-per-surface cadence (the m41→m46 / m45 pattern):
`expectText()` / `expectValue()` compose the identical evaluator
(`clients/typescript/src/expect.ts`) over the identical loop, pinned by
`test/expect.test.ts` and the cross-language e2e leg. No wire change: the
assertions ride existing domains byte-for-byte; the schema, the golden
vectors and the variant tables are untouched. One budget row
(`client_assert`, the satisfied-on-entry round trip over the in-process
door).

---

## 1. Why now

The §8 check vocabulary has run in-process since M0: the kit's
`expect(locator).to_have_text(...)` retries through the zero-polling
waiter, naming the locator and the last evaluation on timeout. Over the
wire the story stopped at states and reads: the m42 wait composed the
event channel with the describe-all re-reads for attached/visible/
hidden/stable, the m46 bulk reads derived the kit's content orders
client-side — but neither *asserts* on content. A remote Rust suite (the
mirrors the TS client has carried since M3, the REPL since M4) wanting
Playwright's `expect(locator).toHaveText(...)` had to poll `describe_all`
by hand with `tokio::time::sleep` between reads — exactly the polling the
event-driven discipline (§15.3, "a waiter fed no events does no work")
exists to prevent. The m42/m45 wait rows covered the state half; the
content half is the same composition one vocabulary deeper, and the m46
module already stated the source orders the assertion compares against.
The milestone fuses them: the wait's wake loop, the bulk module's orders,
one assertion — then carries the same evaluator to the TypeScript client
the way m41/m45 carried the bulk reads and the wait.

## 2. Design: the m46 orders, made waitable

* **Two methods, the two source orders.** `expect_text` and
  `expect_value` mirror the m46 seam pair (`all_text_contents` /
  `all_values`): no wire enum backs this vocabulary, so the kinds live
  client-side — [`ExpectKind::Text`] / [`ExpectKind::Value`] — and the
  m42 `WaitState` rule applies (a client-side vocabulary, never
  serialized). The assertion reuses the m46 derivations verbatim
  (`bulk::text_content` / `bulk::value_content`): one boundary, stated
  once — the fixture's Email input answers `"Email"` to the text read
  and `"ada@example.dev"` to the value read, and neither read can
  silently agree on the wrong datum.
* **The set's head pins the assertion.** Evaluation sees exactly what
  the M39 describe_all walk answers; the assertion reads the set's
  first row (document order — the same pin the m42 wait and the m47
  resolve speak). An empty set stays pending (waitable — nodes stream
  in, Playwright parity), a mismatch stays pending (a re-render can
  restate the content), and the satisfied answer hands back the row
  itself — a directly usable `RequestV1::Action` handle (the dump-row
  rule, M11).
* **The wake loop is the wait's, stated once.** `expect_content`
  composes subscribe → describe-all entry read → wake on the §7 event
  channel → re-read, and the loop body's event half is extracted into
  one shared `next_wake` free function that `wait_for` (m42) now calls
  too — the m46 "shared by every impl" rule applied to the wake loop.
  `Log` records are never wakes; tree and frame wakes both re-decide
  (content has no `Stable`-like frame discipline — every wake re-decides
  the whole read). The subscription window is owned and dropped on every
  exit, the m42 rule verbatim.
* **The honesty is pinned as shipped.** The typed timeout is
  [`GwpError::WaitTimeout`] with the static state word `have text` /
  `have value` (the kit's "stuck on …" convention carried over the wire),
  the locator render, the budget and the last evaluation — expected and
  found, the core `text is …, expected …` wording (§8.3's "failures name
  names"). The match semantics are the core `TextMatch` rules: substring,
  case-insensitive, whitespace-normalized unless `exact` — and `exact`
  is exact-after-normalization (the core rule the tracker tests pin).
  No wire change and no fallback: a driver that predates M39 answers the
  typed `Unsupported` and the assertion surfaces that rejection (the m41
  fail-fast rule).

## 3. Surface

- `crates/guiwright-proto/src/expect.rs` — the [`ExpectKind`] vocabulary,
  the [`ExpectTracker`] evaluator and the [`ExpectEval`] answer.
- `crates/guiwright-proto/src/client.rs` — `ClientConn::expect_text` /
  `expect_value` + the shared `expect_content` composition; the wake
  loop extracted as the shared `next_wake` (m42's `wait_for` calls it
  now too).
- `crates/guiwright-proto/tests/expect_tracker.rs` — the pure evaluator
  pinned without a wire (the `wait_tracker.rs` cases' content mirror).
- `crates/guiwright-driver/tests/wire_client_assert.rs` — the
  conversations: satisfied on entry, the m29/m31 flip pinned over the
  door (a value never satisfies the text read and vice versa), the
  waitable value arriving with a pushed tree update, the typed timeout
  naming locator/expected/found and the empty-set reason, and the
  subscription window dropped on return.
- `crates/guiwright-driver/examples/wire_client_assert.rs` — the
  consumer example over the in-process door (both reads, the waitable
  value change, the typed timeout).
- `crates/guiwright-bench/src/{samples.rs,budgets.rs}` + both
  `budgets.toml` copies — the `client_assert` row.
- `clients/typescript/src/expect.ts` — the TypeScript evaluator (the
  Rust tracker mirrored row-for-row, the Playwright match rule included).
- `clients/typescript/src/client.ts` — `GwpClient.expectText()` /
  `expectValue()` over the shared `expectContent` loop (the m45 wait's
  shape, `log` records never wakes).
- `clients/typescript/test/expect.test.ts` — the pure evaluator's cases
  (wired into `npm test`); `clients/typescript/scripts/e2e.ts` — the
  cross-language leg over real TCP.

## 4. Tests

- `cargo test -p guiwright-proto --test expect_tracker` — 9/9 (the
  orders, the edges, the Playwright match semantics, the set head, the
  empty set, the reason wording).
- `cargo test -p guiwright-driver --test wire_client_assert` — 6/6 over
  the in-process door.
- Red→green: both test files failed to compile against the unfixed seam
  (`could not find expect in guiwright_proto`, `no method named
  expect_text` — the m51 record's red shape), passed after the rows
  landed.
- `cargo run -p guiwright-driver --example wire_client_assert` — the
  demo prints the label-first and value-first reads, the waitable value
  change satisfying after the wake, and the typed timeout naming
  expected and found.
- `cargo test -p guiwright-bench --lib` — the budgets parse with the new
  row bound (the copies-match check green); `check_budgets` reports the
  row OK (median ≈ 73 µs against the 1500 µs design target — the
  subscribe + describe-all + unsubscribe shape, the m42 floor's
  neighbor).
- `npm test` (clients/typescript) — the evaluator's `test/expect.test.ts`
  cases green alongside the codec/client suites (the Rust tracker's cases
  mirrored row-for-row).
- `npm run e2e` — the cross-language leg against the real Rust demo
  server over TCP: both source orders satisfied on entry and the typed
  expiry naming expected and found, the door healthy after the spent
  assertion (`clients/typescript/scripts/e2e.ts`).

## 5. Remaining gaps (honest list)

- No `to_have_count` over the door yet: the kit's count vocabulary
  (M22/M25) composes the same loop over `resolve_all` instead of
  `describe_all`; the seam stays at the content pair until a consumer
  asks (the m48/m51/m52/m53 scope rule).
- No `.not()` modifier on the seam: negation is the kit decorator's
  composition (M24); the seam's honest negative is the typed timeout.
- The kit's `expect(...)` surface is unchanged (in-process, the strict
  gate); the REPL is an unchanged mirror (its `wait`/`texts` rows stay
  the interactive shape); the goldens and the schema are untouched
  byte-for-byte.
- Strict-mode parity is the kit surface's gate: the seam's set reads
  carry the non-strict walk (the m42 rule), so the assertion pins the
  set's head rather than rejecting a multi-match set.
