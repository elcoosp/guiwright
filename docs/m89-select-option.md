# M89 — The select vocabulary: `select_option` lands end to end

**Status:** implemented, green end to end. The m19 record's §16
follow-on list — *"`select_option`, `scroll_into_view_if_needed`,
`drag_to`, `wait_for(state)` and the content getters … stay §16
follow-ons — several need vocabulary the wire does not carry yet
(option lists, scrollable ancestors)"* — has been cashed item by item
as the vocabulary matured (`drag_to` → m33, `wait_for` → m21). This
record pays the first never-cashed item: the §16 actions row's
`select`, the Playwright `locator.selectOption(...)` shape, which needs
exactly the vocabulary the m19 note named — the option rides the wire
as an appended `SemActionV1` variant, the append-only rule keeping every
pinned encoding byte-stable.

---

## 1. Why now

The §16 actions row has carried `select` in its Playwright feature
enumeration since M0, and the adapter machinery has translated the
neighboring semantic actions (`SetValue`, `Check`) since the M0 wire —
but the select vocabulary itself never landed: no `SemAction` variant,
no kit method, no REPL row, no TS table row. The m19 record deferred it
for exactly one reason — the wire did not carry the option — and that
reason dissolved the same way the m64/m66/m68 wire widenings dissolved
theirs: the enum is append-only, a new variant at the end of the table
is free, and the published schema moves with it (codec, TS tables, byte
pins). The m25 comparator note's discipline applies here too: the
vocabulary lands when a suite asks, and the §16 parity row is the ask.

## 2. Design: the option is a string, the translation is per-adapter

Two decisions carry the milestone:

* **The option travels by accessible name.** `SemAction::SelectOption {
  option: String }` carries the same string a text locator matches —
  the a11y-first honest form. No option-list vocabulary enters the
  schema (the m19 note's "option lists" concern stays out of the wire);
  the option is a name, and resolving it is the adapter's job. The
  actionability gate, the auto-wait, the codegen recording and the
  `force`/`timeout_ms` opt-outs ride unchanged — the session's
  `perform_action` path treats the new variant exactly like
  `Check`/`SetValue`.
* **The adapter translation follows §8.4 per framework.** egui and
  masonry take the option as the choice widget's accessible *value*
  (`ActionRequest { SetValue }` with `ActionData::Value` — the same
  request the stock widgets honor for a value change); freya and slint
  ride their fill paths (the value becomes the widget's content); the
  iced engine and the floem/gpui glues answer `AdapterError::Unsupported`
  with the documented reason (the engine carries no option-list op; the
  floem combobox fill focuses the widget the same way the m80
  headless-IME blocker documents; the gpui keystroke-dispatch pipeline
  rides the same m80 scheduler blocker as fill). No silent refusal, no
  stub behavior — the per-adapter caps are the honest §8.4 vocabulary
  the other actions already carry.

## 3. Surface

* `crates/guiwright-driver/src/adapter.rs` — `SemAction::SelectOption`
  + the `describe` arm (`select_option("France")`).
* `crates/guiwright-proto/src/schema.rs` — the appended
  `SemActionV1::SelectOption { option }` variant (discriminant 10, the
  append-only rule's newest row).
* `crates/guiwright-proto/src/convert.rs` — the `ComboBox` role word in
  the wire's closed role table (the select vocabulary's target role;
  unknown names still degrade to `Unknown`).
* `crates/guiwright-driver/src/{server,session}.rs` — the wire
  translation + the codegen verb (`select_option`, the option as the
  recorder argument).
* `crates/guiwright-kit/src/app.rs` — `Locator::select_option(option)`.
* `crates/guiwright-cli/src/repl.rs` — the `select`/`selectoption` row.
* Adapters: egui/masonry (SetValue translation), freya/slint glue
  (fill-path translation), iced engine + floem/gpui glues (typed
  `Unsupported` with the documented blockers).
* `clients/typescript/src/{schema,codec}.ts` — the appended
  discriminant + the `select_option` kind, encode-side.
* Fixture: `Fixture::with_combobox()` (the `NodeId(15)` extension band,
  the M0 base byte-stable).

## 4. The evidence

* **Kit (`guiwright-kit/tests/locator_actions.rs`, +2):** the red step —
  `locator.select_option("France")` records
  `SemAction::SelectOption` at the adapter and the codegen program
  carries `.select_option("France");` — both green after the
  vocabulary landed.
* **Wire (goldens):** `action_select_option` joins the golden vectors;
  `GUIWRIGHT_BLESS=1` re-pinned `clients/typescript/test/golden.json`,
  the TS codec test builds the request and asserts byte-identity
  (`codec.test.ts` — the m89 pin joining the M30/M32/M64… pins).
* **REPL (`guiwright-cli/tests/repl_e2e.rs`, +1):** a real UDS session —
  `get role ComboBox Country` resolves the combobox (the new role word),
  `select "France"` renders the `ActionDone` detail verbatim, the
  missing-argument refusal is one error line.
* **Example (`guiwright-kit/examples/select_option.rs`):** the run
  prints the adapter detail, the recorded codegen line and the trace
  path — `cargo run -p guiwright-kit --example select_option`.
* No budget row: the action rides the existing semantic-action budget
  rows (the m54-family rule — vocabulary, not a new hot path).

## 5. What this does not claim

* No option *list* read on the wire: a suite enumerating an open
  combobox's choices rides the existing tree reads (`describe_all` over
  the options' subtree) — the select vocabulary answers "choose this
  one", not "what are the choices".
* The iced/floem/gpui rows stay the documented `Unsupported` caps (the
  honest per-adapter vocabulary; the upstream blockers are recorded in
  m80's honest list).
* The TS client gains no dedicated `selectOption()` sugar: the typed
  `action(node, generation, action, force, timeout)` row (M47) speaks
  the appended variant directly, codec-side — the per-surface sugar is
  its own milestone when a remote TS suite asks (the m64/m65 pattern).
