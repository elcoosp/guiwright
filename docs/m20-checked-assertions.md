# M20 — Checked-state assertions on the expect surface

**Status:** implemented (this milestone cashes the "checked-state
assertions" item the M19 record explicitly deferred: the wire's `Check`
semantic action was actionable everywhere, but nothing could *read* the
resulting state — `expect(locator).to_be_checked()` needed the tri-state
surfaced through the §8 check vocabulary first, which is core-side work
that did not belong inside the M19 action scope).

---

## 1. Why now

The §16 assertions row carried `toBeChecked` as an unshipped Playwright
parity item since M0. Everything downstream of the check already existed:
the wire speaks `SemAction::Check/Uncheck` (actionable from the kit since
M19), and AccessKit models the result as the tri-state
`Toggled::{True, False, Mixed}` on every tree node. The only missing piece
was the read side: no `Check` row evaluated `toggled`, so no assertion
could wait on it. M20 lands that one seam and the whole assertion family
rides the existing waiter machinery unchanged.

## 2. Design: one check row, three assertion names

The load-bearing decision mirrors M19's: **no new session, wire or schema
surface**. The tri-state becomes one new actionability row,
`Check::Checked(Toggled)`, evaluated exactly like its neighbors — a pure
function of the consumer tree, `Pending` while the state may still flip,
and re-evaluated on every tree diff the session pumps (§15.3 zero-polling).

- **The state is waitable, the role is not.** A checkbox whose state has
  not flipped yet stays `Pending` (Playwright parity: retry until the
  state holds or the timeout expires). A node that never exposes a
  checked state (`toggled() == None` — a plain button) fails **fast**:
  the world cannot change it, so burning the five-second default timeout
  would be a lie. `CheckStatus::Failed` short-circuits the wait.
- **Three builder names, one check row.** `to_be_checked()`
  (`Toggled::True`), `to_be_unchecked()` (`Toggled::False`) and
  `to_be_checked_mixed()` (`Toggled::Mixed`) — the paired-negation shape
  the builder already uses for visible/hidden and enabled/disabled, with
  the mixed tri-state spelled out instead of a parameter enum.
- **Timeout errors name the assertion and the locator.** The expect
  builders (core, driver session and kit) now render failures as
  `checked — getByRole(CheckBox, "Dark mode" (fuzzy))` — the waiter's own
  description already carried both, and the re-wrap kept only the
  assertion name. Nothing pinned the shorter form; the locator belongs in
  the error (§8.3 "the error embeds the last tree snapshot" — and names
  what was being waited for).

## 3. Surface

- `crates/guiwright-core/src/actionability.rs` — the `Check::Checked`
  row + `checked_status` (pass / pending / fail-fast) and the
  `describe` renderings ("checked", "unchecked", "mixed-checked").
- `crates/guiwright-core/src/expect.rs` — `ExpectBuilder::to_be_checked`
  / `to_be_unchecked` / `to_be_checked_mixed`; `Expectation` carries the
  `name — locator` failure header.
- `crates/guiwright-core/src/test_util.rs` — the `with_checkboxes()`
  fixture (the `buttons()` base plus a checked / unchecked / mixed trio;
  `buttons()` keeps its 8-node shape byte-stable) and the
  `checkbox_toggle_update` helper for mid-wait flips.
- `crates/guiwright-driver/src/session.rs` — the same three methods on
  the session `ExpectBuilder` (the remote-facing assertion path) plus the
  failure-header fix.
- `crates/guiwright-kit/src/app.rs` — the kit `Pending` pass-throughs and
  the previously session-only `with_timeout` override.
- `crates/guiwright-bench` — the `checked_assert` row: one strict resolve
  + checked evaluation on the trio fixture (measured ≈1 µs against the
  100 µs p50 design row — the floor every `to_*` adds on top of a
  resolve).

## 4. Tests

- `crates/guiwright-core/src/actionability.rs` — the tri-state rows, the
  pending-until-flip rule and the fail-fast rule.
- `crates/guiwright-core/src/expect.rs` — the three assertions plus both
  failure shapes (wrong state times out with the state named; non-checkable
  role refuses).
- `crates/guiwright-driver/src/session.rs` — the session-level trio, the
  timeout wording, the mid-wait flip completing on the scripted tree diff
  (the shared-script channel), and the fail-fast path.
- `crates/guiwright-kit/tests/checked_assertions.rs` — the kit surface
  end-to-end: the trio, the timeout naming, the fail-fast path.
- `crates/guiwright-kit/examples/checked_assertions.rs` — the consumer
  example: all three states asserted, then both failure shapes printed.

## 5. Remaining gaps (honest list)

- Checked-state *reads* on the remote path (a `TreeDump` row or getter
  carrying `toggled` for the TypeScript client) stay §7 follow-ons — the
  wire's dump vocabulary does not carry the tri-state yet, and this
  milestone deliberately stayed out of the schema.
- `setChecked`-state change events (assert via subscription instead of
  polling the tree) would ride the M12 event channel with a new
  `EventKindV1` — same schema scope, same reasoning.
