# M94 — The REPL chain tokens: the `All` descendant chain rides the prompt

**Status:** implemented, green end to end. The m93 record's honest list
carried one named gap: *"No REPL chain tokens: the `All` descendant
chain stays a kit/TS-side composition (`locator(child)`); the REPL
renders a pasted chain row (`a >> b`) but speaks no syntax for building
one — the narrowing words are the prompt's vocabulary, the chain is the
DSL's."* This record pays it: the `>>` binder joins full locator
segments into the wire's `All` chain, so the interactive half of the
§8.1 chain vocabulary lands — and m93 §1's own claim ("every `Selector`
the engine resolves is now reachable from the REPL") is true without
the asterisk.

---

## 1. Why now

The kit locator DSL has composed descendant chains since M34
(`locator(child)`), and m93 gave the REPL the narrowing words — but the
chain form stayed DSL-only. A debugging session aiming "the Save button
inside the group" at the prompt had no way to say it: `count role Group
>> role Button` joined `>> role Button` as the role *name* (the
recorded red step — the same failure mode the m93 record pinned for the
filter words). Meanwhile the prompt could already *render* a pasted
chain row (`a >> b`) — the grammar could speak a shape it could not
build. Every other surface the plan traces composes chains; the prompt
was the last one without them.

## 2. Design: the binder is a split, the resolver keeps the semantics

Two decisions carry the milestone:

* **One seam — the shared get-args parse, one level up.** The `>>`
  binder splits the get-args into full locator segments *before* the
  m93 filter fold runs, and each segment parses through the same base +
  filter-word path — so every get-args command speaks chains in one
  pass (the reads, the strict `get`, the `assert` rows), no per-command
  parsing, no new commands. The segments fold left-to-right into the
  wire's `All(Vec<SelectorV1>)` in composition order — source first,
  the descendants after, the kit `locator(child)` shape (M34). One
  segment answers exactly the selector the grammar always spoke; the
  `All` row is the chain's own wire shape, not a wrapper.
* **No new wire shape, no new semantics — and one resolver honesty fix
  the grammar forced.** `SelectorV1::All` shipped with M34 and every
  client already speaks it; the resolver owns the subtree walks. But
  the chain grammar is the first producer of *composite chain links*
  (`Group >> Button has_text Save` is `All([Group, Filter{…}])`), and
  the resolver's `matches_selector` answered a composite with the
  subtree walk it delegates from — `matches_selector` ↔
  `subtree_matches_selector` recursing on the same selector, a stack
  overflow no test had ever reached (the m93 renders could carry a
  pasted chain, but no wire producer sent one as a link). The fix is
  the honest semantics split, at the seam: `matches_selector` answers
  composites with node-level membership (`key` is one of the matches
  the selector resolves to — an ancestor of a match is not itself a
  match, the m34 has/chain distinction); `subtree_matches_selector`
  answers them with one structural resolution and a containment check
  (which matches land in the subtree), never a per-descendant
  re-resolution. Leaf rows keep the existing walk. `has`/`has_not`
  inners gain the same totality for free (a chain inner resolves, no
  recursion). The renders speak the grammar's own words — what you
  typed is what the describe prints (`Group >> Button has_text
  "Save"`) — and the m35 rules carry unchanged: the reads stay reads,
  an empty chained set is a valid answer, and `>>` binds at chain
  level (a `has`-inner carries no chain; a name carrying `>>` needs a
  different phrasing — the same reserved-word trade-off family the
  m93 grammar documented).

## 3. Surface

- `crates/guiwright-cli/src/repl.rs` — `CHAIN_TOKEN` + `split_chain`
  (the binder's split and its error usage), `parse_selector` over the
  segment fold (base + filter words per segment, the `All` wrap in
  composition order), and the help row carrying the binder.
- `crates/guiwright-core/src/tree.rs` — the composite arms of
  `matches_selector` (node-level membership) and
  `subtree_matches_selector` (one resolution + `subtree_contains`),
  the resolver-side seam the chain links ride.
- `crates/guiwright-cli/examples/repl_chain_rows.rs` — the consumer
  demo: the ancestor/chain distinction at the prompt, the leaf rule at
  chain width, filters inside segments and before the binder, the
  strict get + click loop aiming the chained resolve, a count
  assertion riding the chain, and a probe re-answered by the scripted
  update's tree event (no polling) — `cargo run -p guiwright-cli
  --example repl_chain_rows`.

## 4. Tests

- `crates/guiwright-core/tests/selectors.rs` — `composite_link_in_chain_resolves`
  (the chained filter resolves Save, strictly), 
  `composite_chain_answers_the_link_matches_only` (the ancestor is
  never a match), `composite_inner_in_has_resolves` (the chain inner
  under `has` resolves, the honest empty read). **Red→green recorded**:
  the composite tests abort with a stack overflow against the unfixed
  resolver (`matches_selector` ↔ `subtree_matches_selector` on the
  same composite); the membership/containment split passes them, and
  the 20-test file rides green including `fuzz_selectors_never_panic`.
- `crates/guiwright-cli/tests/repl_e2e.rs` —
  `repl_chain_rows_in_get_args` over a real UDS socket: the
  ancestor/chain contrast (`has` answers the group, `>>` answers the
  buttons), the leaf rule at chain width, filters inside segments, the
  `has`-before-binder fold, the strict `get` + click loop (the action
  row rendering the composed describe), the `assert count` row
  resolving through the chain, the error honesty (a leading, a
  trailing and a doubled `>>` all name the usage, the session
  survives), and the help row. **Red→green recorded**: the test fails
  against the unfixed REPL (`0 matches (Group ">> role Button")` — the
  binder joined as the role name), passes after the grammar lands.
- No wire change, no golden, no TS row: the `All` encoding was pinned
  by the Rust side first (M34) and every client already speaks it.
- No budget row (the m35 REPL-pass rule, carried by m61/m74/m93): the
  rows are client-side composition over the already-budgeted resolve /
  resolve-all / assert conversations — a second row would measure the
  same conversation twice.

## 5. Remaining gaps (honest list)

- No regex `TextPattern` at the prompt: the m34 record's standing note
  keeps its own trigger (the enum stays open until the codegen milestone
  asks — the codegen milestones shipped without asking), so the
  chain segments' text probes speak the fuzzy `TextMatch` default only.
- `>>` binds at chain level: a `has`/`has_not` inner carries no chain
  and a quoted name carrying `>>` parses as the binder — the grammar
  documents the trade-off instead of adding escape syntax (the kit DSL
  and the TS helper have no such collision).
- No REPL chain *abbreviation*: every segment repeats its locator kind
  (`role Group >> role Button`); a bare-word continuation (`>> Button`)
  would need a kind default the grammar does not carry — the kit DSL's
  arguments are values, the REPL's are tokens, and the chain rides the
  same explicitness rule the base vocabulary speaks.
