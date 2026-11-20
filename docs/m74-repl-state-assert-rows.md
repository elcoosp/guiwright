# M74 — The REPL's waitable state assertions (`assert checked` / `assert enabled` / `assert focused`)

**Status:** implemented. The m73 seam assertions' interactive half (the
m43/m61/m62 one-milestone-per-surface pattern): the kit's
`to_be_checked` / `to_be_enabled` / `to_be_focused` vocabulary at the
prompt — `assert checked <get-args…> is checked|unchecked|mixed|not-checkable`
/ `assert enabled <get-args…> is enabled|disabled` / `assert focused
<get-args…>` riding the [`ClientConn::expect_checked`] /
[`ClientConn::expect_enabled`] / [`ClientConn::expect_focused`] seam
rows (the m54 wake loop over the m70 set-width state read,
event-driven, no polling), the satisfying m68 row rendered with the
state three named. **No wire change**: the rows compose the m73
assertions client-side; every `ClientConn` impl inherits them. No
budget row (the m54/m59/m62/m64/m66/m68/m70/m72/m73 rule: the row is
the seam loop's prompt face over the already-budgeted state-walk
conversation shape).

---

## 1. Why now

The state facts became waitable over the door in m73 and readable at
the prompt in m68's `describe` render — but not assertable at the
prompt. The m61/m62/m64 `assert` grammar carried the content, count and
identity families to the REPL; the m73 record named the gap honestly:
"the `assert` grammar's state kinds (`assert checked` / `assert
enabled` / `assert focused`) are their own milestone (the m43/m61/m62
interactive-half pattern)". This record cashes it: a scripted spec or a
debugging human can now ask the door "wait until this checkbox is
checked" in the same grammar the content and identity assertions speak.

## 2. Design: the m61 grammar at state width

* **One kind word per seam row** — the rows map 1:1 onto the GWP
  surface the m43 rule demands: `assert checked` calls
  `expect_checked`, `assert enabled` calls `expect_enabled`,
  `assert focused` calls `expect_focused`. No new wire domain, no new
  request — the grammar is sugar over the m73 rows.
* **The four-way word rides verbatim** — the checked demand speaks the
  m68 vocabulary the render prints, kebab-cased to one token:
  `checked` / `unchecked` / `mixed` / `not-checkable`. A mixed checkbox
  never satisfies an unchecked demand, and `not-checkable` answers as
  the total fact (never an error — the m68 rule at waitable width, now
  at the prompt). The enabled demand speaks the pair the render prints:
  `enabled` / `disabled`.
* **`focused` takes no expectation** — the fact itself is the demand;
  the seam's typed timeout is the honest negative (the m73 no-negation
  rule carried to the prompt). A stray `is` token is a grammar error
  naming the row's shape, not a silently ignored suffix.
* **The render is the m68 describe render** — the satisfying row
  answers as one ok line rendered by
  `render_identity_state` (the m68 facts plus `state focused|unfocused
  enabled|disabled <checked-word>`), so the prompt speaks the same
  words the `describe` row does — the fact the assertion waited for is
  named on arrival.
* **The failures name names** (§8.3): a spent budget prints the seam's
  typed timeout verbatim — `wait be checked timed out after <ms> …
  (checked is Checked, expected Unchecked)` — the m61 rule unchanged.
  The empty set stays pending (the m54 empty-content rule's shape: no
  node, no fact, no satisfaction), so the spent budget names the empty
  set.
* **A read, not a resolve-store** — `last` keeps the node `get` aimed
  at, and the assertion's subscription window restores the `events on`
  filter (the m43 rule, shared with every `assert` row).

## 3. Surface

- `crates/guiwright-cli/src/repl.rs` — the three dispatch arms and the
  `assert_checked_command` / `assert_enabled_command` /
  `assert_focused_command` rows, the `parse_checked_word` /
  `parse_enabled_word` expectation parsers, the `state_line` render
  (the m68 row at the prompt), the `ASSERT_CHECKED_USAGE` /
  `ASSERT_ENABLED_USAGE` / `ASSERT_FOCUSED_USAGE` grammar consts beside
  the m61 usage table, the kinds usage line extended
  (`… description | checked | enabled | focused`), and the help rows.
- `crates/guiwright-cli/tests/repl_e2e.rs` — the wire conversations
  over a real socket.
- `crates/guiwright-cli/examples/repl_assert_state_rows.rs` — the
  consumer demo.

## 4. Tests

- Red→green: the red step ran the new suites against the missing rows
  — both failed on `error: unknown kind \`checked\`` (the append-only
  discipline breaking the runtime on purpose), the rows the green.
- `cargo test -p guiwright-cli --test repl_e2e` — 23 passing (was 21):
  the four-way word demanded exactly on entry (checked, unchecked,
  mixed, and the not-checkable total fact), the enabled pair both ways,
  the seeded focus answering the window row, the grammar validation
  naming each row's usage, the typed timeouts (`be checked` /
  `be enabled` / `be focused` + the fact found), the empty set pending,
  the wake-and-re-read on the scripted `Toggled::True` flip and the
  scripted focus arrival, the `events on` filter restored, and the
  read-not-store rule.
- The demo conversation is runnable: `cargo run -p guiwright-cli
  --example repl_assert_state_rows` prints the tri-state asserted on
  entry, the waited flip, the typed timeout and the surviving session.
- No budget row: the m54-family rule carried forward (the row rides the
  budgeted state-walk conversation shape; the grammar adds no wire cost
  to guard).

## 5. Remaining gaps (honest list)

- The TypeScript mirror is its own milestone (the m45/m60/m63/m65/m67/
  m69/m71 one-milestone-per-surface pattern) — landed as the m75
  record in the same stream.
- No kit change: the m68 rule stands — the kit's `to_be_*` vocabulary
  keeps its embedded core-wait semantics; the seam rows are the remote
  half, the m72 reads the total face. Nothing in `guiwright-kit` moved.
- No negation on the seam: the honest negative is the typed timeout
  (the m54/m59/m62/m64/m67/m68/m70/m72/m73 rule, carried unchanged).
- `assert focused` has no unfocused complement: demanding the *absence*
  of focus would be a negation row, and the seam's rule names the typed
  timeout as the negative instead.
