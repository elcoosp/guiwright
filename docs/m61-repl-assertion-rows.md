# M61 — The REPL assertion rows (`assert text|value|count <get-args…> is <expected>`)

**Status:** implemented. Three REPL command rows over the m54/m59 client-seam
assertions — the interactive half of the seam, the m43/m45
one-milestone-per-surface pattern: `assert text|value <get-args…> is
<expected>` rides `ClientConn::expect_text` / `expect_value`, `assert count
<get-args…> is <N>` / `is > <N>` / `is < <N>` rides
`ClientConn::expect_count` and the M25 comparators, all with the
`timeout <ms>` budget. No wire change (the assertions compose existing
domains byte-for-byte), no session change, no budget row (the m35/m40/m43
rule restated: the rows are client-side composition over the
already-budgeted `client_assert` / `client_count_assert` conversations — a
second row would measure the same round trips twice).

---

## 1. Why now

The m59 and m60 records both named the gap: "no count row in the REPL: the
kit, the seam and the TS client carry the family; the REPL's `count` read
(m35) stays a read, an `expect count` row would be the m43
interactive-half pattern's follow-up." The shape repeats the m41/m42
history — the embedded kit has driven the waitable `expect(locator).to_*`
vocabulary since M20/M22, the seam carried it over any door since m54/m59,
and a debugging session at the REPL still stopped at snapshot reads: run
`count`, re-run, eyeball, repeat. The seam landed first, so the rows cost
one command arm plus the composition the four shipped clients already
inherit.

## 2. Design: the seam's rows, the REPL's grammar

* **The rows land under their own `assert` word — not `expect`.** `expect`
  already owns the runner-wording resolve (`expect <get-args…>`, M5) and
  `text` / `value` / `count` are locator kinds in the get vocabulary — one
  word cannot lead two grammars without re-wording the scripted rows (the
  m43 append-only rule). The m59 record's "`expect count` row" phrasing
  names the capability, not the word; the honest delimiter is a new command.
* **The first standalone `is` token separates the locator from the
  expectation.** The locator vocabulary is greedy (multi-word names join —
  `role Button Save All`), so a word is the delimiter that keeps
  `assert text role Button Save is "Save All"` parseable without quoting
  rules on the left side.
* **A satisfied assertion answers one ok line naming the satisfying fact** —
  the m37 row render for the content pair (`assert text — Button "Save"
  (generation 0) rect=[…]`, directly aimable by the action vocabulary) and
  `assert count — found 4 (Button)` for the count family (the m59 answer
  shape; absence answering found 0 is an answer like any other, Playwright
  parity).
* **A spent budget prints the typed timeout verbatim** — `wait have text
  timed out after 150 ms — getByRole(…) (text is "Save", expected "Ghost"
  (fuzzy))`: the seam's state word, the locator render, the last evaluation
  (expected and found; the failures-name-names discipline at the interactive
  surface). The timeout line's `wait` prefix is the typed error's own
  Display — the same words the m43 row prints, one rendering to maintain.
* **A read, not a resolve-store** (the m35 rule, unchanged): the rows never
  move `last` — `get` → `assert count` → `last` still aims the get's node.
* **The matcher rides the REPL's posture** — substring, case-insensitive
  (the `match_arg` shape every `get`/`label` word speaks); the expected role
  rides the get vocabulary's first-letter rule (`canonical_role`), the role
  comparing exactly on the seam (a structural fact).
* **The subscription window restores the `events on` filter** (best
  effort) — the m43 rule, now extracted into the shared `restore_events`
  helper the wait row and the assertion rows both call, so the interactive
  flow keeps flowing after any seam row's window. The unsupported-log-tail
  case is a no-op by construction (the guard keys on the REPL's own
  `events_subscribed` state).
* **Validation names the grammar** — one error line per mistake (`assert`
  without a kind, an unknown kind, a missing `is`, an unknown comparator),
  the session usable after (the survival rule).

## 3. Surface

- `crates/guiwright-cli/src/repl.rs` — the `assert` dispatch arm;
  `assert_command` + the content row (`assert_content_command`) and the
  count row (`assert_count_command`, the comparator parse in
  `parse_count_spec`); the `is` separator (`split_at_is` / `assert_tail`);
  the shared `restore_events` helper (extracted from the m43 wait row); the
  command-list rows.
- `crates/guiwright-cli/tests/repl_e2e.rs` — the wire conversations (below).
- `crates/guiwright-cli/examples/repl_assert_rows.rs` — the consumer demo
  (the wake conversation, runnable).

## 4. Tests

- `repl_e2e.rs` — `repl_assert_rows_wait_on_the_client_seam` over a real
  UDS socket (the log tail enabled, the m43 test's server shape), in four
  phases: the satisfied forms (content text/value with the fuzzy matcher,
  the count exact form and both comparators, absence answering found 0, the
  read-not-store rule); the grammar validation (one error line per mistake,
  the session usable); the typed timeouts at the 150 ms budget (`have text`
  / `have count`, the expected/found sides, the empty-set pending); and the
  wake-and-re-read on a scripted fifth button (event-driven, no polling)
  with the `events on` filter restored after (the restore) — plus the help
  listing the rows and the legacy `expect` row untouched (append-only).
- Red→green: the suite failed against the unfixed REPL (unknown command
  `assert` — the m35 record's red shape), passed after the arms landed;
  the wake phase re-run for the event-flow timing.
- The demo conversation is runnable:
  `cargo run -p guiwright-cli --example repl_assert_rows` prints the
  satisfied count, the wake-and-re-read (4 → 5 on the scripted update), the
  content rows, the typed timeout and the surviving session.

## 5. Remaining gaps (honest list)

- No TS row and no wire domain: the assertions are client-side
  compositions; the TypeScript client's assertion surface is unchanged
  (m60's list carries).
- No negation at the REPL: `.not()` stays the kit decorator's composition
  (M24); the seam's honest negative is the typed timeout (the m54/m59 rule,
  carried unchanged).
- The REPL `count` read (m35) stays a read: `assert count` is the waitable
  row, `count` the snapshot read — the vocabulary keeps both shapes, the
  same distinction m43 drew between `wait` and `get`.
