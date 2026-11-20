# M72 — The kit's identity-state reads (`describe_state` / `describe_all_state`)

**Status:** implemented. The m68/m70/m71 records' named standing gap,
cashed: "a kit-side `describe_state` / `describe_all_state` read lands
when a consumer asks." The state three — focused, enabled, the checked
tri-state — ride the kit locator surface at both widths:
`Locator::describe_state()` resolves through strict mode and answers the
handle-width row (`Session::describe_state`, M68), and
`Locator::describe_all_state()` walks the set (`Session::describe_all_state`,
M70) answering the tri-state per row in document order. **No wire
change**: the wire's `DescribeIdentityState` / `DescribeAllIdentityState`
encodings (discriminants 22/25, 23/26) stay byte-pinned — the kit is the
embedded door, and the remote door has carried the rows since m68/m70.
No budget row (the m54/m62/m64/m66/m68/m70 rule carried forward: the
reads compose the budgeted describe conversations' shapes, the embedded
door pays no wire cost).

---

## 1. Why now

The wire has carried the state facts at both widths since m68 (handle)
and m70 (set), and the assertion vocabulary has answered state questions
*waitably* since M20 (`to_be_checked` / `to_be_focused` /
`to_be_enabled`) — but the kit's describe surface stopped at the
identity row: `describe()` (M36) and `describe_all()` (M39) carry role,
name, value, bounds and never the state three. A suite auditing an
embedded app asking "what is this checkbox **doing**" either fired the
assertion vocabulary one fact at a time (each a wait loop, when the
audit wants a total fact) or dropped to `Session::describe_state`
directly (under the locator surface, losing strict mode, narrowing and
the window pin). The m68 record reserved the kit read ("a kit-side
`describe_state` read lands when a consumer asks"), the m70/m71 records
carried the note unchanged, and the consumer that asks is the same audit
walk m70 named: a batch read of the tri-state per row beside the
identity facts, in the pass the set read already costs.

## 2. Design: the m68 total-read rule at the locator surface

* **The state read is a total read, never a wait** — the m68 rule
  carried to the kit unchanged: the vocabulary words answer right now
  (`NotCheckable` for a role that exposes no checked state, never an
  error), and the waitable face stays the kit's `to_be_checked` /
  `to_be_focused` / `to_be_enabled` assertions. The two faces compose:
  an audit asserts waitably when the world may still move, and reads
  totally when it wants the fact as it stands.
* **`describe_state` resolves through the strict-mode surface** — the
  `describe()` shape verbatim: resolve (strict mode, the same
  surface every locator read aims through, the window pin narrowing the
  walk), then `Session::describe_state(&resolved.key,
  resolved.generation)`. The generation guard is the boundary's: a
  recycled id answers `StaleNode`, a dead one `UnknownNode`, and the
  kit maps both through the existing `KitError` conversion — the
  error-context convention the describe read already rides.
* **`describe_all_state` keeps the snapshot semantics** — the
  `describe_all()` rule verbatim: no strict mode, no waiting, an
  empty set answers with an empty list, a window-pinned locator reads
  only inside its subtree, and a read records nothing (resolution
  without an action is not a replayable step, so codegen stays silent —
  the m36 rule).
* **The row is the driver's, not a reshape** — the kit returns
  `guiwright_driver::NodeStateDescription` (the identity row beside
  the state three, the m68 compose-don't-widen shape at kit width), now
  re-exported beside `NodeDescription` so consumers name one path. The
  checked fact is the wire's `CheckedStateV1` four-way word — a mixed
  checkbox is not an unchecked one.
* **No REPL row, no TS mirror.** The one-milestone-per-surface
  discipline: this record is the kit face. The REPL's interactive face
  at handle width already renders the state three (m68), the m70 record
  named the set walk's no-single-answer shape, and the TS client
  mirrors `ClientConn` rows, not kit surfaces — a kit-shaped TS surface
  is not a named gap.

## 3. Surface

- `crates/guiwright-kit/src/app.rs` — the `Locator::describe_state()`
  row (the `describe` shape: strict-mode resolve + the M68 session
  read) and the `Locator::describe_all_state()` row (the
  `describe_all` shape: the snapshot walk + the M70 session read), the
  rustdocs citing the m68 total-read rule and the m39/m70 snapshot
  semantics.
- `crates/guiwright-driver/src/lib.rs` — the `NodeStateDescription`
  re-export beside `NodeDescription` (the kit's rustdoc link and the
  consumers' one path; the type itself is the m68 session row,
  untouched).
- `crates/guiwright-kit/Cargo.toml` — the `guiwright-proto` dev-only
  dependency (the state tests name the `CheckedStateV1` word the facts
  ride; schema types, not runtime plumbing).
- `crates/guiwright-kit/tests/describe_state.rs` — the wire-free kit
  conversations: the trio's tri-state per row in document order, the
  identity row riding beside the state three, the button's
  `NotCheckable` fact, strict mode and `.first()` narrowing, the
  window pin at both widths, the honest empty list, the missing match's
  typed failure.
- `crates/guiwright-kit/examples/describe_state_facts.rs` +
  `crates/guiwright-kit/examples/describe_all_state_rows.rs` — the
  consumer demos (the m68 handle-width facts; the m70 set walk).

## 4. Tests

- Red→green: the compile was the red step (the m43/m64/m66/m68 kit-half
  shape — `describe_state` / `describe_all_state` did not exist on the
  locator surface; the suite failed against the missing methods, the
  append-only discipline breaking the compile on purpose), the rows the
  green.
- `cargo test -p guiwright-kit --test describe_state` — 9 passing: the
  trio's `Checked` / `Unchecked` / `Mixed` at handle width, the
  `NotCheckable` button fact, the identity row riding beside (bounds +
  generation), strict mode refusing the bare role locator, `.first()`
  answering document order, the missing match's typed error, the window
  pin at both widths (the described node's key tree == the pinned
  window), the walk's tri-state in document order, the honest empty
  list.
- The demos are runnable: `cargo run -p guiwright-kit --example
  describe_state_facts` prints the trio's tri-state plus the button's
  `NotCheckable`; `cargo run -p guiwright-kit --example
  describe_all_state_rows` prints the walk's tri-state per row in
  document order, the narrowed first row and the honest empty list.
- No budget row: the m54/m62/m64/m66/m68/m70 rule carried forward (the
  reads compose the describe conversations' shapes; the embedded door
  pays no wire cost to guard).

## 5. Remaining gaps (honest list)

- No negation on the seam: `.not()` is the kit decorator's composition
  (M24); the state reads' honest negative is the typed error (the
  m54/m59/m64/m67/m68/m70 rule, carried unchanged).
- No kit *assertion* over the state rows: the m68 rule stands — the
  waitable face is the `to_be_*` vocabulary, the state read is total;
  nothing changed here.
- No REPL row, no TS mirror: the one-milestone-per-surface discipline
  (§2); neither is a named standing gap in the records.
