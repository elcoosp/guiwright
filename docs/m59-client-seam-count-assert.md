# M59 — The client seam's count assertion (`ClientConn::expect_count`)

**Status:** implemented. Three default methods on the [`ClientConn`] seam
(`crates/guiwright-proto/src/client.rs`) — the kit's `to_have_count`
family (the M22 exact form + the M25 one-sided comparators) as typed
sugar, the one-milestone-per-surface pattern (m46→…→m58) carried to the
count domain. A downstream Rust consumer asserts a set's size with one
call and receives the found count, instead of hand-composing
`subscribe` / `resolve_all` / wake loops. The m54 record's named gap
closed: *"No `to_have_count` over the door yet: the kit's count
vocabulary (M22/M25) composes the same loop over `resolve_all` instead
of `describe_all`."* No wire change: the assertion composes the existing
`ResolveAll` and event domains byte-for-byte as M12/M30 blessed them;
the schema, the golden vectors and the variant tables are untouched.
One budget row (`client_count_assert`, the satisfied-on-entry floor over
the in-process door).

---

## 1. Why now

The seam's assertion half landed with m54 — `expect_text` /
`expect_value` made the kit's content pair waitable over any door — and
its own record named what stayed raw: the count vocabulary. The kit has
asserted set sizes since M22 (`to_have_count`, the strict-mode escape
hatch) and one-sided sizes since M25 (`to_have_count_greater_than` /
`to_have_count_less_than`), but a remote suite had to hand-compose the
m42 wake loop over the M30 set read to say "expect four buttons". Every
other surface rides the seam now; the count family was the last
assertion vocabulary the door could not speak. The m54 shape applies
with one substitution: the evaluation rides `resolve_all` instead of
`describe_all` — a count is a fact about the set, so the M30 set read is
the honest round trip (the smaller rows, no per-row facts marshalled for
an answer that only reads their length).

## 2. Design: the count family, as sugar

* **Three rows, one composition.** `expect_count(selector, expected,
  timeout)` answers the kit's M22 row; `expect_count_greater_than` /
  `expect_count_less_than` answer the M25 comparators. All three are
  one-liners over a private `expect_count_with(selector, CountMatch,
  timeout)` — the m54 `expect_content` shape (thin named rows over one
  shared loop), the comparison carried as the core's own
  [`CountMatch`] vocabulary (the m48 take-the-existing-vocabulary rule;
  no parallel enum invented client-side).
* **The answer is the found count.** The satisfied assertion hands back
  `usize` — the fact the assertion speaks (the m57 summary shape). The
  set's rows themselves ride `resolve_all` (M30): one read per question,
  the rows usable as action handles where the m30/m47 rows already
  publish that rule.
* **Absence is an answer.** An empty set is found 0 — satisfying
  `count 0` and every less-than (Playwright's `toHaveCount(0)` parity),
  unlike the content pair's empty-set pending (`crate::expect`'s
  boundary). This is the one semantic place the count evaluation
  diverges from m54, and the tracker's own tests pin it.
* **The predicate is the core's.** `CountMatch::holds` published with
  this milestone (`guiwright-core/src/wait.rs`): the comparison was
  already core's vocabulary, so its predicate is the honest shared shape
  (the m46 derivation-module rule — one boundary, stated once). The
  evaluator (`crates/guiwright-proto/src/count.rs`, `CountTracker`) is a
  pure, transport-blind core: set in, `CountEval` out, no cross-wake
  state (a count has no `Stable`-like frame discipline — every wake
  re-decides the whole read, the m54 rule).
* **Failures name names.** The typed timeout (`GwpError::WaitTimeout`)
  names the state word `have count`, the locator render, the budget and
  the last evaluation — "count is 4, expected 99" / "> 99" / "< 2" —
  never a bare "timed out" (§8.3, the m54 wording shape).
* **No wire change and no new capability flags.** The assertion rides
  the `ResolveAll` + event domains byte-for-byte; a driver that predates
  M30 answers the set read with the typed `Unsupported` and the
  assertion surfaces that rejection rather than degrading (the m41
  fail-fast rule, restated for counts).

## 3. Surface

- `crates/guiwright-proto/src/count.rs` — the `CountTracker` evaluator
  (new module: vocabulary, set evaluation, the state word, unit tests).
- `crates/guiwright-proto/src/client.rs` — `ClientConn::expect_count` /
  `expect_count_greater_than` / `expect_count_less_than` + the private
  `expect_count_with` composition + the `expire_count` timeout builder.
- `crates/guiwright-proto/src/lib.rs` — the module bullet.
- `crates/guiwright-core/src/wait.rs` — `CountMatch::holds` published.
- `crates/guiwright-driver/tests/wire_client_count.rs` — the
  conversations: satisfied on entry (whole set + narrowed), the
  comparators' right and wrong sides, absence as an answer, the
  waitable growth (a fifth button joins), the timeout naming the
  comparison and the found count, the subscription window dropped.
- `crates/guiwright-driver/examples/wire_client_count.rs` — the consumer
  example over the in-process door.
- `crates/guiwright-bench/src/{samples.rs,budgets.rs}` + both
  `budgets.toml` copies — the `client_count_assert` row.
- `docs/architecture.md` + `README.md` — the §15.7 row count (61), the
  parity-map m59 sentence, the status-table M59 row and the records
  range.

## 4. Tests

- `cargo test -p guiwright-proto --lib` — the tracker's unit pins: the
  exact form answers the set length and names both sides, an empty set
  is found 0 (satisfying `count 0` and every less-than, pending a
  greater-than), the comparators answer the same found fact, the state
  word is stable.
- `cargo test -p guiwright-driver --test wire_client_count` — 6/6 over
  the in-process door.
- Red→green: the first conversation test failed against the unfixed
  seam (`no method named expect_count` — the m51/m55/m56/m57/m58
  record's red shape) and went green after the three rows landed.
- `cargo run -p guiwright-driver --example wire_client_count` — the demo
  prints the exact read, the waited growth (`found 5 (satisfied after
  the wake)`), both comparator forms and the typed timeout line.
- `cargo test -p guiwright-bench --lib` — the budgets parse with the new
  row bound (the copies-match check green); `check_budgets` reports the
  row OK (each measured iteration is one satisfied assertion — the
  subscribe + resolve-all + unsubscribe floor, no wake riding the
  number).

## 5. Remaining gaps (honest list)

- No negation on the seam: `.not()` is the kit decorator's composition
  (M24); the seam's honest negative is the typed timeout (the m54
  record's rule, carried unchanged).
- No TypeScript row yet: the TS client's `expectText()` / `expectValue()`
  (m54's cross-language half) has no `expectCount()` mirror — the
  one-milestone-per-surface pattern's natural follow-up, riding the same
  resolve-all composition (no protocol change either side).
- No count row in the REPL: the kit and seam surfaces carry the family;
  the REPL's `count` read (m35) stays a read, an `expect count` row
  would be the m43 interactive-half pattern's follow-up.
- The kit's `expect(...)` surface is unchanged (in-process, the strict
  gate); the goldens and the schema are untouched byte-for-byte.
