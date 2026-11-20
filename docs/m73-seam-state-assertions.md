# M73 — The seam's waitable state assertions (`expect_checked` / `expect_enabled` / `expect_focused`)

**Status:** implemented. The state family's waitable half over the
door: the kit's `to_be_checked` / `to_be_enabled` /
`to_be_focused` vocabulary (M20 and the core check family) as waitable
assertions over any door — `ClientConn::expect_checked(selector,
expected, timeout)` / `expect_enabled(selector, expected, timeout)` /
`expect_focused(selector, timeout)` composing the m54 wake loop with the
m70 set-width state read (`describe_all_identity_state` re-reads), the
satisfying m68 row handed back as the dump-row handle. **No wire
change**: the assertion rides existing domains byte-for-byte (the m68
`DescribeIdentityState` and m70 `DescribeAllIdentityState` encodings
stay byte-pinned); every `ClientConn` impl inherits the rows. No budget
row (the m54/m59/m62/m64/m66/m68/m70/m72 rule: the assertion is the m54
loop's edges over the already-budgeted state-walk conversation shape).

---

## 1. Why now

The state facts became waitable at the kit surface years before they
existed over the door: `to_be_checked` / `to_be_focused` /
`to_be_enabled` ride the embedded core's waiter state machines (M20+),
while every remote seam row that answered a state question —
`describe_identity_state` (m68), `describe_all_identity_state` (m70) —
is a total read by the m68 rule. A remote suite (the TypeScript client,
the REPL, any `ClientConn` consumer) asking "wait until this checkbox
is checked" had no row: the m54 content family made text waitable over
the door, the m59 count family made set sizes waitable, the m62/m64
identity family made role/name/description waitable — the state three
stayed read-only. This record closes that half-door the same way the
m54 record closed the content one: the facts were already on the wire,
the wake loop already existed, and the composition is client-side only.

## 2. Design: the m54 loop's edges at state width

* **The composition is `expect_content`'s, verbatim, one read wider** —
  subscribe (`TreeChanged` + `FramePresented`), evaluate the set's head,
  wake on the §7 event channel, re-read
  `describe_all_identity_state(selector)` on every wake, and hand back
  the satisfying m68 row on success. Tree and frame wakes both
  re-decide (the state facts have no `Stable`-like frame discipline);
  `Log` records are never wakes; the subscription window is owned and
  dropped on both the satisfied and the expiry path (`expire_state`,
  the `expire_expect` helper's shape).
* **The kinds are a client-side vocabulary, never serialized** — the
  `WaitState`/`ExpectKind` rule restated: `StateKind::Checked(
  CheckedStateV1)` / `Enabled(bool)` / `Focused` compose the existing
  domains byte-for-byte, so the published schema and the TS byte pins
  do not move.
* **The tri-state is exact** — the m68 word demanded as the word: a
  mixed checkbox never satisfies a `Checked` demand, `Unchecked` never
  satisfies it, and `NotCheckable` stays pending (the total fact, not
  an error — the m68 rule at waitable width). `Enabled(bool)` is the
  `to_be_enabled` / `to_be_disabled` pair in one word; `Focused` waits
  for the focus to arrive (the core `Check::Focused` read's shape —
  `tree.focus()` equality).
* **An empty set stays pending** — the m54 empty-content rule's shape
  (waitable — nodes stream in), unlike the count family's
  absence-is-an-answer (m59): no node, no fact, no satisfaction.
* **The failures name names** (§8.3): the typed
  [`GwpError::WaitTimeout`] carries the state word (`be checked` / `be
  enabled` / `be focused`), the locator render, the budget and the
  last evaluation — the fact read and the fact wanted.
* **No REPL row, no TS mirror.** The one-milestone-per-surface
  discipline: this record is the `ClientConn` face. The REPL `assert`
  grammar's state rows and the TypeScript `expectChecked()` /
  `expectEnabled()` / `expectFocused()` mirrors are their own
  milestones (the m45/m60/m61/m62 pattern) — neither is a named
  standing gap until a consumer asks.

## 3. Surface

- `crates/guiwright-proto/src/expect.rs` — the `StateKind` vocabulary
  (`Checked` / `Enabled` / `Focused` + the `state()` timeout words),
  the `StateEval` answers, the `StateTracker` row-set evaluator (the
  `ExpectTracker` shape: no cross-wake state, every wake re-decides).
- `crates/guiwright-proto/src/client.rs` — the `expect_checked` /
  `expect_enabled` / `expect_focused` rows and the shared
  `expect_state` composition (the `expect_content` loop over the m70
  state read), the `expire_state` timeout helper beside
  `expire_expect`.
- `crates/guiwright-driver/tests/wire_client_assert_state.rs` — the
  wire conversations over the in-process door: the entry-satisfied
  checked assertion, the scripted flip the assertion waits for
  (event-driven, no polling), the enabled boolean both ways with the
  typed timeout naming the fact, the focus arriving with its update,
  and the unfocused budget naming `focused is false`.
- `crates/guiwright-driver/examples/wire_client_assert_state.rs` — the
  consumer demo (the tri-state asserted on entry, the waited flip, the
  focus arrival, the typed timeout).

## 4. Tests

- Red→green: the compile was the red step (the m54/m62/m64 seam-half
  shape — `expect_checked` / `StateTracker` / `StateKind` did not
  exist; the suites failed against the missing rows, the append-only
  discipline breaking the compile on purpose), the rows the green.
- `cargo test -p guiwright-proto --test expect_tracker` — 19 passing
  (was 14): the checked word exact (a mixed checkbox is not an
  unchecked one; `NotCheckable` stays pending), the enabled boolean
  both directions, the focused fact, the empty set pending on every
  kind, and the pending reason naming expected and found.
- `cargo test -p guiwright-driver --test wire_client_assert_state` — 6
  passing: `expect_checked` satisfied on entry (the row as the
  dump-row handle), the scripted `Toggled::True` flip the assertion
  waits out (event-driven), the wrong-word typed timeout (`be
  checked` / `Unchecked` named), `expect_enabled` both ways with the
  honest negative, the scripted focus arrival, and the unfocused
  budget naming `focused is false`.
- The demo conversation is runnable: `cargo run -p guiwright-driver
  --example wire_client_assert_state` prints the entry assertion, the
  waited flip, the focus arrival and the typed timeout line.
- No budget row: the m54/m62/m64/m66/m68/m70/m72 rule carried forward
  (the assertion rides the budgeted state-walk conversation shape; the
  composition adds no wire cost to guard).

## 5. Remaining gaps (honest list)

- No TypeScript mirror: `expectChecked()` / `expectEnabled()` /
  `expectFocused()` are their own milestone (the m45/m60/m63/m65/m67/m69
  one-milestone-per-surface pattern) — the codec, the variant tables and
  the byte pins already moved so the next record's red step is the
  client methods, not the codec.
- No REPL rows: the `assert` grammar's state kinds (`assert checked` /
  `assert enabled` / `assert focused`) are their own milestone (the
  m43/m61/m62 interactive-half pattern).
- No kit change: the m68 rule stands — the kit's `to_be_*` vocabulary
  keeps its embedded core-wait semantics; the seam rows are the remote
  half, and the m72 reads are the total face. Nothing in
  `guiwright-kit` moved here.
- No negation on the seam: the honest negative is the typed timeout
  (the m54/m59/m62/m64/m67/m68/m70/m72 rule, carried unchanged).
