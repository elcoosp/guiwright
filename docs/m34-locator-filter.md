# M34 — The locator filter surface (`has_not` + chained narrowing)

**Status:** implemented. Two kit locator methods over the core resolver's
existing `Filter { has_not }` / `All(chain)` machinery — the Playwright
`filter({ hasNot })` and chained `locator(selector)` shapes, the §8.1
selector vocabulary completed end-to-end — one composition-order core
sugar, one kit-side budget row. No wire change, no session change (the
proto's `SelectorV1` and the convert tables already carry the full
filter vocabulary; the kit locator was the only consumer surface that
could not reach it).

---

## 1. Why now

The §8.1 selector model has always carried the full narrowing
vocabulary: `Selector::Filter { has_not }` and the chained `All`
descendant chain are resolved by the core engine (`filter_candidates` /
`all_candidates`), published on the wire (`SelectorV1`'s filter rows,
the convert tables), and covered by core's own selector tests. But the
kit locator — the consumer surface every suite actually writes —
exposed only `first` / `last` / `nth` / `has_text`. A suite that
needed the inverted filter (the classic "every row except the selected
one") or the descendant chain (`locator.locator(child)`) had to
hand-build a `Selector` value and leave the locator DSL entirely —
losing the auto-wait shape, the window pin ergonomics and the
borrow-and-act doctrine at exactly the compositional step suites use
most. The m22 count record called strict mode "a set-size question"
and pointed at narrowing for the escape hatch; with `has_not` missing,
half that hatch was behind a raw-selector wall. This milestone closes
the vocabulary: every `Selector` the engine resolves is now reachable
from the locator surface.

## 2. Design: pure composition, the inversion made honest

Two decisions carry the milestone:

* **No engine change — the kit composes by value.** `has_not(inner)`
  and `locator(child)` rebuild the locator's selector through the
  existing core sugars (`Selector::has_not`, the new
  `Selector::then_descendant`); the resolver nests `Filter` over `All`
  over `Filter` already (the recursion in `matches_selector`), so
  composition is free and the strict-mode resolve, the auto-wait, the
  window pin and the actionability gate ride unchanged. The only core
  addition is the constructor in composition order:
  `then_descendant(inner)` pushes `inner` onto the receiver's chain
  (`parent.then_descendant(child)` = `parent >> child`), and
  `descendant_of` becomes its argument-flipped delegate — behavior
  identical, the "is a descendant of" reading preserved.
* **The inversion is stated, not implied.** `has_not` keeps matches
  whose subtree — the node included — matches no node of the inner
  selector, the same subtree walk `has_text` reads in the positive.
  That rule is load-bearing and surprising at the leaf: a leaf's
  subtree is itself, so `get_by_role(Button).has_not(&button)`
  excludes every button — `has_not(button)` keeps only the containers
  that hold no button. The rustdoc states it, the test pins it, and
  the inner selector is a full `Selector` (a role+name constraint, a
  test id, a nested chain), not a text probe.

The borrow rule stays untouched: both methods take `&Selector` and
clone (the `drag_to` target doctrine — every locator holds the `&mut
App` it acts through, so no locator-valued argument can exist). An
empty chain answer stays the valid snapshot read (`count()` answers
0); a failed act-time resolve is the strict-mode error naming the
miss.

## 3. Surface

- `crates/guiwright-core/src/selector.rs` — `Selector::then_descendant`
  (the composition-order chain sugar; `descendant_of` refactored as its
  argument-flipped delegate, zero behavior change).
- `crates/guiwright-kit/src/app.rs` — `Locator::has_not(inner:
  &Selector)` and `Locator::locator(child: &Selector)` (the rustdocs
  carry the subtree-included rule and the composition semantics).

## 4. Tests

- `crates/guiwright-kit/tests/locator_filter.rs` — the surface
  end-to-end on the mock adapter: the inverted filter dropping the
  group whose subtree fits the inner selector (role+name and role
  forms), the leaf pin (a button filtered by `has_not(button)` excludes
  itself; a label stays), the `group >> Button` chain counts (4 → named
  1 → ghost 0), the chain + `first` document-order resolve pinned
  against `key_of`, composition over the chain (`has_not` and
  `has_text` after `locator`), the chained act running the full strict
  gate (one adapter action, the 4-match ambiguity refused with nothing
  synthesized), and the window pin scoping the composed set on the
  two-window fixture (2 unscoped, 1 per window, keys pinned against the
  fixture's `w1_save_key`/`w2_save_key`).
- Red→green: the test file fails to compile against the unfixed kit
  (16 `no method named has_not/locator` errors), passes 7/7 after the
  two methods land.
- `bench/budgets.toml` + `crates/guiwright-bench/src/budgets.toml` —
  the `locator_filter` row (sample: one composed `group >> Button`,
  `has_not(Confirm)` set read — 3 matches, the chain walk plus the
  inverted-filter probes), enforced by the CI guard; the crate copy
  byte-identical (`crate_copy_matches_canonical`).

## 5. Remaining gaps (honest list)

- No `filter({ has })` positive-locator filter on the kit surface yet
  (`has_text` covers the string form; the selector-valued positive
  filter is `All`-chain shaped and reachable via `locator()` for the
  chain case — a distinct `has(...)` sugar lands when a suite asks).
- No regex `TextPattern` (the selector module's standing note — the
  enum stays open until the codegen milestone asks).
- The wire carries the full filter vocabulary unchanged (no new
  protocol surface here), so other-language clients keep composing
  filters through the raw selector rows — a typed TS `filter()` helper
  stays with the TS client's own pass.
