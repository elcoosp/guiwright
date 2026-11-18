# M28 — The editable/read-only assertion pair (`to_be_editable` / `to_be_readonly`)

**Status:** implemented. Two checks on the §8 check vocabulary, two
assertion methods on the driver builder and the kit surface, one fixture
extension, one budget row — no new waiter, no protocol change (the
standing assertion-milestone shape: the `expect` vocabulary is
client-side by design).

---

## 1. Why now

The M20 checked-state work extended the §8 check vocabulary with the
AccessKit tri-state; the editable dimension is the other per-node state
Playwright asserts that the vocabulary did not yet speak. AccessKit
already carries everything the consumer needs: the `read_only` flag, the
role defaults (`should_have_read_only_state_by_default`) and the
read-only-supported set, folded into one total predicate —
`accesskit_consumer::NodeRef::is_read_only()`. Guiwright had no way to
name it from a test: `to_be_enabled` speaks enablement, `to_be_checked`
speaks toggling, and editability stayed unreachable. This milestone
surfaces the predicate as the check vocabulary's next negated pair —
the same shape as the M20 checked row (a state check rides an existing
waiter, no new machinery).

## 2. Design: one effective predicate, two check arms

The load-bearing decision: **the check speaks the consumer's *effective*
read-only state, not the raw flag.** `NodeRef::is_read_only()` folds
three sources — the explicit `read_only` flag, role defaults, and the
rule that a role which does not support read-only (a button has no
editable value) reports read-only — into one total predicate. Building
on the effective state buys two properties:

* **Totality.** Every node answers; there is no fail-fast arm (contrast
  the M20 checked check, whose non-checkable roles fail fast). Playwright
  parity follows directly: a wrong state is waitable (apps flip the flag
  mid-session), and a button reads read-only exactly as
  `expect(button).not().toBeEditable()` does.
* **Negation composes for free.** `Check::Editable` /
  `Check::ReadOnly` are the same predicate inverted, so
  `expect(locator).not().to_be_editable()` (the M24 decorator) and the
  direct `to_be_readonly()` agree by construction — one predicate, two
  spellings.

The timeout header keeps the established checks-waiter shape
(`editable — getByRole(…)`, the M20/M22 convention); the pending reason
("node is read-only") rides the waiter's describe path, not the header
(`ChecksWaiter::stuck_on` stays `None`, the standing M27 shape).

## 3. Surface

- `crates/guiwright-core/src/actionability.rs` — `Check::Editable` /
  `Check::ReadOnly` (describe + evaluate arms) and `editable_status`
  (the one predicate, `want_editable` parameterized).
- `crates/guiwright-core/src/test_util.rs` — `Fixture::with_readonly()`
  (the `buttons()` base plus "Bio" `NodeId(12)`, editable, and "Serial"
  `NodeId(13)`, `set_read_only`; ids 8 and 9..=11 stay reserved for
  `add_button_update` / `with_checkboxes`) and
  `Fixture::input_readonly_update(node, read_only)` (the mid-wait flip).
- `crates/guiwright-driver/src/session.rs` — `ExpectBuilder::to_be_editable`
  / `to_be_readonly` (the shared `run` path; negation and window scopes
  ride unchanged).
- `crates/guiwright-kit/src/app.rs` — `Pending::to_be_editable` /
  `to_be_readonly` (the kit surface).

## 4. Tests

- `crates/guiwright-core/src/actionability.rs` (inline) — the predicate
  table: both inputs pass their direction, wrong directions pend, a
  button reports read-only, `describe` renders `editable` / `read-only`.
- `crates/guiwright-kit/tests/editable_assertions.rs` — end-to-end
  through the session loop: both directions, the `not()` composition,
  the waitable timeout headers naming assertion + locator, the scripted
  `read_only` flag flip completing mid-wait on the tree-diff event, and
  the button effective-state rule.
- `crates/guiwright-kit/examples/editable_assertions.rs` — the consumer
  example: both directions on the input pair, the button rule, the
  negation composition, and the failure shape printed.
- `bench/budgets.toml` + `crates/guiwright-bench/src/budgets.toml` —
  the `editable_assert` row (sample: one `to_be_editable()` over the
  read-only fixture's editable input — strict resolve + the effective-
  state evaluation, the checked_assert row's shape), enforced by the CI
  guard.

## 5. Remaining gaps (honest list)

- No wire domain / REPL row — the assertion vocabulary stays client-side
  by design (the standing deferred-half shape of the assertion
  milestones; the wire's `Resolve`/`Action` domains never carried
  `expect`).
- `to_be_empty` (the third Playwright state assertion in this family:
  no text and no value) is a label/value composite, not an effective-
  state predicate — it lands on its own milestone if a suite asks for
  it, riding the `HasText` machinery rather than this check's.
- Codegen records `to_be_editable()` like every `to_*` — no new cost
  model rule was needed (unique names and `.nth()` pins behave as
  everywhere else).
