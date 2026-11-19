# M62 — The client seam's identity-assertion rows (`expect_role` / `expect_name`)

**Status:** implemented. Two `ClientConn` default methods over the m54 wake
loop — the kit's `to_have_role` / `to_have_accessible_name` vocabulary (M38)
over any door, the m54/m59 one-milestone-per-surface pattern, composed with
the m61 REPL rows as their interactive face. **No wire change**: the rows
compose the existing `DescribeAll` + event domains byte-for-byte (the M39
describe-all rows already carry the identity facts); every [`ClientConn`]
impl inherits them. No budget row (the m54 rule: the content assertions'
composition rides the already-budgeted `describe_all_wire` /
`client_assert` conversations — the identity rows are the same loop with a
different predicate).

---

## 1. Why now

The kit has carried the identity family since M38 (`to_have_role` /
`to_have_accessible_name` / `to_have_accessible_description`); the client
seam's assertion vocabulary stopped at the content pair and the counts
(m54/m59) — a remote suite could wait for a row's text and a set's size,
but not for "this control is the structural kind I expect" or "this label
is what it says it is" over a door. The m61 REPL rows were the consumer
that asked: the assertion vocabulary at the prompt wants the identity facts
the M39 walk already carries, and the seam's scope rule ("until a consumer
asks") had its ask.

## 2. Design: the m38 vocabulary over the m54 loop

* **The kinds ride the same `ExpectKind` enum, never serialized** (the
  m42 `WaitState` rule): `Role(String)` compares the head row's role
  exactly — a structural fact takes no matcher, no fuzz; the AccessKit
  debug name the describe rows print (`Button`, `TextInput`) is the
  vocabulary the role locators speak. `Name(TextMatchV1)` reads the
  label-only accessible name — narrower than the text kind's m29 order
  (the value never answers), the fact the text row cannot pin.
* **The state words** are `have role` / `have name` (the kit's stuck-on
  convention carried over the wire, the m54 `have text` shape) — the typed
  timeout names them, the locator render and the last evaluation (the
  failures-name-names discipline): `role is "Button", expected
  "TextInput"`; `name is "Save", expected "Ghost" (fuzzy)`.
* **A node with no accessible name stays pending** (waitable), the reason
  naming which fact is missing — the m54 empty-content rule's shape at the
  single-row width.
* **The set's head pins** (the m54 rule, unchanged): document order, first
  row; later rows never answer. An empty set stays pending (the m54
  entry-evaluation wording) — unlike the count family's absence-is-an-answer.
* **The REPL consumes the rows** (`assert role` / `assert name`, the m61
  grammar): the expected role rides the get vocabulary's first-letter rule,
  the expected name the REPL's matcher posture; the satisfying row renders
  as the m37 facts (the dump-row rule).
* **The evaluator stays pure and transport-blind**: the per-kind evaluation
  lives in `expect.rs` beside the content kinds (the content pair's verdict
  factored into one `content_verdict` helper both arms call), the
  composition stays in the trait (subscribe → describe-all re-reads → wake
  on the §7 channel, the subscription window owned and dropped) — the same
  `expect_content` loop the m54 rows ride, no second copy to maintain.

## 3. Surface

- `crates/guiwright-proto/src/expect.rs` — the `Role` / `Name` kinds, the
  per-kind evaluation, the shared `content_verdict` helper.
- `crates/guiwright-proto/src/client.rs` — the two `ClientConn` rows
  (`expect_role` / `expect_name`) composing `expect_content`.
- `crates/guiwright-cli/src/repl.rs` — the `assert role` / `assert name`
  rows (the m61 grammar), the kinds usage string, the command-list row.
- `crates/guiwright-driver/tests/wire_client_identity.rs` — the wire
  conversations (below).
- `crates/guiwright-driver/examples/wire_client_identity.rs` — the consumer
  demo (the rename wake conversation, runnable).

## 4. Tests

- `wire_client_identity.rs` over the in-process door with the
  shared-script channel standing in for app-side UI changes: the satisfied
  forms (the head row's role answering the exact string, the label-only
  name answering the matcher); the typed timeouts naming `have role` /
  `have name` with the found and expected sides; the missing-name pending
  (the unnamed group cannot satisfy a name assertion, the reason naming the
  missing fact); and the wake-and-re-read on a scripted rename (the fuzzy
  locator rides the rename — the head row stays the Save button across it).
- Red→green: the compile was the red step (the m43 seam-half shape — the
  methods did not exist), the rows the green.
- The REPL conversations ride `repl_e2e.rs`
  `repl_assert_rows_wait_on_the_client_seam`: the `assert role` /
  `assert name` satisfied forms, the `have role` timeout at the REPL, the
  kinds usage string and the command-list row (the m61 test grew the
  identity phases).
- The demo conversation is runnable:
  `cargo run -p guiwright-driver --example wire_client_identity` prints the
  role row, the label-only name row, the rename wake and the typed timeout.

## 5. Remaining gaps (honest list)

- No description row: the M39 domain does not carry the description fact
  (`DescribedV1` has role / name / value / bounds), so
  `to_have_accessible_description` cannot ride without a wire change —
  reserved until a consumer asks (the append-only protocol rule).
- No TypeScript row: the TS mirror is its own milestone (the m45/m60
  one-milestone-per-surface pattern).
- No negation on the seam: `.not()` is the kit decorator's composition
  (M24); the seam's honest negative is the typed timeout (the m54/m59 rule,
  carried unchanged).
