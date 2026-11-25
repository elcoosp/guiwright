# M93 — The REPL filter grammar: the §8.1 narrowing words ride every locator

**Status:** implemented, green end to end. The m91 record's honest list
carried one named follow-on: *"No REPL row: the REPL's get-args
vocabulary speaks no filter rows today (`has_not` included) — the m34
surface drew the same line, and the REPL filter grammar stays its own
pass if a debugging session asks."* The m92 record drew the same line
first ("the REPL's get-args vocabulary speaks no filter rows (`has_not`
included) — the m91 record drew the same line"). This record pays it:
the interactive half of the §8.1 filter vocabulary, the m35/m40/m43/m61/
m74 one-pass-per-surface pattern carried to the narrowing words.

---

## 1. Why now

The kit locator DSL has spoken the full narrowing vocabulary since
m34/m91 (`has_text`, `has_not`, the chained `locator`, the selector-valued
`has`, the nth forms), and the TypeScript client gained its typed
`filter()` helper with m92 — but the REPL's get-args vocabulary still
parsed exactly one selector form (`role | text | label | testid | value`).
A debugging session composing "the buttons without a Confirm" or "the
group that holds a Save" at the prompt had no narrowing words at all:
`count role Button has_text Save` joined `has_text Save` as the role
*name* (the recorded red step). Every other consumer surface the plan
traces had its filter pass; the prompt — the place a human pokes a live
app — was the last one without it. This record closes the vocabulary:
every `Selector` the engine resolves is now reachable from the REPL.

## 2. Design: the words are postfix, the resolver keeps the semantics

Two decisions carry the milestone:

* **One seam — the shared get-args parse.** The grammar extends
  `parse_selector`, the one function every get-args command funnels
  through (`get`, `count`, `all`, `texts`, `values`, `describe`,
  `snapshot under`, `wait`, the `assert` rows, `expect`, `inspect`), so
  every command speaks the words in one pass — no per-command parsing,
  no new commands. The base locator tokens run to the first reserved
  word (`has_text`, `has_not`, `has`, `first`, `last`, `nth`); the words
  fold left-to-right after it, each wrapping the selector so far in its
  own wire row — source first, narrowing after, the kit locator DSL's
  composition order (`Locator::has_text(..).first()`). `has`/`has_not`
  take a full locator, so they recurse on the remainder and end the
  fold (the wire nests `Filter` over `Has` over `Filter` already). The
  reserved words win over name tokens — a node named `first` needs a
  different phrasing, the same trade-off every token grammar makes (the
  kit DSL has none; its arguments are values, the REPL's are tokens).
* **No new wire shape, no new semantics.** Each word speaks a row the
  wire has carried since M34/m91 (`Filter { has_text, has_not, nth }`,
  `Has { source, has }`) — the resolver owns the subtree walks, the
  leaf rule and the document-order nth pick, and every session rides
  them already. The REPL composes rows and renders; it re-decides
  nothing. The renders speak the grammar's own words (what you typed is
  what the describe prints: `Button has_text "Save"`, `Group has
  Button`, `Button nth 1`); the core render's Playwright words
  (`filter(hasText=…)`) stay the codegen side. The m35 rules carry
  unchanged: the reads stay reads (no resolve-store), an empty narrowed
  set is a valid answer, and a driver predating the domains degrades by
  the established fallbacks.

## 3. Surface

- `crates/guiwright-cli/src/repl.rs` — `FILTER_WORDS` + the split/fold
  (`filter_word`, `parse_selector` over the base arms and
  `fold_filters`), the `render_selector` `Filter`/`Has`/`All` arms, and
  the help row carrying the words.
- `crates/guiwright-cli/examples/repl_filter_rows.rs` — the consumer
  demo: the narrowed reads, the leaf rule at the prompt, the strict get
  + click loop aiming the filtered resolve, a count assertion riding
  the words, and a probe re-answered by the scripted update's tree
  event (no polling) — `cargo run -p guiwright-cli --example
  repl_filter_rows`.

## 4. Tests

- `crates/guiwright-cli/tests/repl_e2e.rs` —
  `repl_filter_rows_in_get_args` over a real UDS socket: the `has_text`
  probe (the fuzzy default, the empty set read), the `has` ancestor
  keep, the `has_not` inverted filter + the leaf rule at the prompt,
  the three nth forms in document order, the fold's composition order,
  the strict `get` + click loop (the action row rendering the composed
  describe), the `assert count` row resolving through the words, the
  error honesty (every filter word demands its argument, a reserved
  word alone is not a locator), and the help row. **Red→green
  recorded**: the test fails against the unfixed REPL (`0 matches
  (Button "has_text Save")` — the words joined as the role name), passes
  27/27 after the grammar lands.
- No wire change, no golden, no TS row: the `Filter`/`Has` encodings
  were pinned by the Rust side first (m34/m91) and every client already
  speaks them.
- No budget row (the m35 REPL-pass rule, carried by m61/m74): the rows
  are client-side composition over the already-budgeted resolve /
  resolve-all / assert conversations — a second row would measure the
  same conversation twice.

## 5. Remaining gaps (honest list)

- No regex `TextPattern` at the prompt: the m34 record's standing note
  keeps its own trigger (the enum stays open until the codegen milestone
  asks — the codegen milestones shipped without asking), so the REPL's
  `has_text` word speaks the fuzzy `TextMatch` default only.
- No REPL chain tokens: the `All` descendant chain stays a kit/TS-side
  composition (`locator(child)`); the REPL renders a pasted chain row
  (`a >> b`) but speaks no syntax for building one — the narrowing
  words are the prompt's vocabulary, the chain is the DSL's.
- Reserved-word collisions: a quoted name carrying `has_text`/`first`/
  `nth` parses as the filter word, not the name — the grammar documents
  the trade-off instead of adding escape syntax; the kit DSL and the TS
  helper have no such collision.
