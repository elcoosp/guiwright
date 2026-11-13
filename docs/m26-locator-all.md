# M26 — The locator `all()` enumeration

**Status:** implemented (the §16 locators row's enumeration half: the
m22 and m25 records both left `locator.all()` open as "a kit
borrow-shape design question" — this closes it, on the existing
`resolve_all` seam, with no new session, wire or core machinery).

---

## 1. Why now

M22 shipped the set-size *read* (`locator.count()`) beside the
`to_have_count` assertion, and the m25 record closed by naming the
other half of the set surface as the remaining open gap:
`locator.all()` — Playwright's `Locator.all()`, the enumerate-the-
matches surface every iterate-and-act flow wants (click every row that
matches, collect the identity of every dialog). The blocker was never
the semantics; it was the borrow shape the m22 record flagged: a kit
[`Locator`] holds the `&mut App` it acts through, so a Playwright-style
`Vec<Locator>` cannot exist — two locators would be two mutable
borrows of one app. The enumeration needed a shape that fits
borrow-and-act, not a new resolution path.

## 2. Design: an iteration seam, not a vector

The load-bearing decision: **the matches arrive one at a time**.
`Locator::all` takes a visitor and hands it each match as an
nth-pinned [`Locator`], collecting the visitor's per-item results:

```rust
let keys = app.role_locator(Role::Button)
    .all(|item| Ok(item.resolve()?.key))?;   // Vec<NodeKey>
```

`FnMut(Locator<'_, A>)` means each visit reborrows the app for exactly
the call's duration — the borrow the previous item held ends with its
call, so the next one re-aims cleanly. The generic return is the data
escape hatch: the visitor maps each match to whatever the flow needs,
so the whole-set read (`all(|item| Ok(item.resolve()?.key))`) and the
iterate-and-act flow (`all(|item| { item.click()?; Ok(()) })`) are the
same seam.

Everything else follows the standing contracts:

- **the set is a snapshot read** — one scoped `resolve_all` walk (the
  `count()` read generalized to identities), no waiting: an empty set
  enumerates to zero visits, and a node that arrives later shows up on
  the next enumeration (Playwright parity — `all()` does not wait);
- **each item is `locator.nth(i)`** — the visited locator re-resolves
  at act time and auto-waits like any locator, so a match removed
  mid-iteration fails that item's action instead of visiting a stale
  node, and the visited selector renders as `getByRole(Button).nth(0)`
  (the pin is real, not a bookkeeping detail);
- **narrowing composes** — a `.first()` / `.nth(2)` / `.has_text(…)`
  locator enumerates its own narrowed set (the same set `count()`
  reads, because both ride `resolve_all`, which applies the filter
  chain);
- **the window pin rides every item** — a window-pinned locator
  enumerates only inside its subtree, the M6 scoping semantics;
- **recording is unchanged** — enumeration itself is a read (the
  `count()` rule: nothing recorded), while actions on the visited
  items record exactly like the equivalent hand-written `.nth(i)`
  actions; the §13 cost model still picks the minimal stable selector
  from the resolved node's facts.

No wire, session or core changes: the enumeration lives entirely on
the kit surface, reusing `run_scoped`, `resolve_all` and the existing
`Nth` filter vocabulary.

## 3. Surface

- `crates/guiwright-kit/src/app.rs` — `Locator::all` (the iteration
  seam, with the borrow-shape rationale in the rustdocs).

## 4. Tests

- `crates/guiwright-kit/tests/all_matches.rs` — end-to-end on the mock
  adapter: document order over the four-button fixture, the nth pin on
  every visited selector (and stability across passes), the empty set,
  the late-arriving node joining the next enumeration, the window pin
  scoping the set (both saves tree-wide, exactly one inside Preview),
  per-item data collection through the generic return, and the
  read-not-action recording rule with a visited click recording the
  minimal stable selector.
- `crates/guiwright-kit/examples/all_matches.rs` — the consumer
  example: immediate enumeration in document order, identity
  collection, a visited `fill`, a scripted late button growing the set
  between enumerations, and the window-scoped enumeration on the
  two-window fixture.
- `bench/budgets.toml` + `crates/guiwright-bench/src/budgets.toml` —
  the `locator_all` row (sample: one `role_locator(Button).all(…)`
  over the button fixture — the `resolve_all` set read plus one
  index-pinned strict resolve per visited item), enforced by the CI
  guard.

## 5. Remaining gaps (honest list)

- Bulk *content* reads (`all_text_contents()`-shaped helpers) are not
  part of this milestone — the visitor can collect per-item reads
  today; a dedicated bulk reader lands when a suite asks for it.
- No wire domain / REPL row — reads and enumeration are client-side by
  design (the standing deferred-half shape of the assertion
  milestones); over GWP the enumeration would ride the same
  `Resolve`-reply extension the count surface names.
- Codegen records the minimal stable selector, not the visited item's
  `.nth(i)` pin — the standing §13 cost-model rule (unique names
  survive re-runs better than positional pins), unchanged here on
  purpose.
