# M31 — The value-first bulk read (`all_values`)

**Status:** implemented. One kit method over the existing
`resolve_all` seam — the m29 record's named follow-up ("a value-first
bulk read for form dumps is a one-line variation of the same walk —
deliberately not added until a suite asks for it"). No new session
API, no waiter, no wire change (reads are client-side by design, the
m26 record's standing note; the set read's wire half shipped as M30).

---

## 1. Why now

M29 closed the bulk *content* read with an explicit named gap: the
text-first reader answers what the text locators match (label first —
the `HasText` check's source order), and a **form dump** wants the
other datum — what the form *carries*. The record deliberately held
`all_values()` back "until a suite asks for it"; with M30 landing the
set read's wire half and the M29 record naming this as the one-line
variation, the reader completes the read family: `count()` (the set
size), `all()` (the identities), `all_text_contents()` (the text),
`all_values()` (the values).

## 2. Design: the same walk, the source order flipped

The load-bearing decision: **the flip is the whole feature.** The read
rides the identical non-strict `resolve_all` walk `count()`, `all()`
and `all_text_contents()` take; each match contributes `value` first,
falling back to `label` — the `getByValue` source order. That buys:

* **The two reads can never silently agree on the wrong datum.** The
  fixture's Email input reads `Email` through `all_text_contents` and
  `ada@example.dev` through `all_values` — the label-first order is
  the text locators' contract, the value-first order is the form
  dump's, and neither borrows the other's semantics.
* **Totality.** Every node answers: a node with neither value nor
  label reads as an empty string, keeping `Vec` positions 1:1 with
  the document order the set read established (the m29 positional
  stability rule, unchanged).
* **Set semantics for free.** Hidden nodes are included, the window
  pin scopes the walk, narrowing scopes the read, a late-arriving
  node joins the next read — the snapshot-read family rules, shared
  verbatim.

The read records nothing: the `count()` rule (resolution without an
action is not a replayable step) extends to content reads — no codegen
line, no trace action. No session method was needed:
`Session::resolve_all` and the tree's value/label accessors were
already public, so the read stays a kit-side composition.

## 3. Surface

- `crates/guiwright-kit/src/app.rs` — `Locator::all_values` (the
  value-first reader, with the source-order rationale in the
  rustdocs).

## 4. Tests

- `crates/guiwright-kit/tests/all_values.rs` — end-to-end on the mock
  adapter: the Email input reading its value (not its label — the
  flip, pinned against `all_text_contents`'s documented behavior),
  the label fallback over the valueless buttons (document order,
  hidden included), the empty set, `.first()` narrowing, the window
  pin scoping the read (the two-"Save" discriminator fixture), and
  the neither-node reading as an empty string (the unlabeled group).
- `crates/guiwright-kit/examples/all_values.rs` — the consumer
  example: the fallback set, the value-first input read side by side
  with the text-first read, the narrowed set, the window-scoped read,
  and the codegen nothing-recorded rule printed.
- `bench/budgets.toml` + `crates/guiwright-bench/src/budgets.toml` —
  the `all_values` row (sample: one `all_values()` over the button
  fixture — the `text_contents` row's walk with the flipped source
  order), enforced by the CI guard.

## 5. Remaining gaps (honest list)

- No wire domain / REPL row — the read rides `resolve_all`
  client-side by design; over the wire the M30 `ResolveAll` domain
  returns identities + rects, and bulk *content* stays with the tree
  dump (the m29/M30 records' shape question, unchanged).
- Multi-line / descendant text runs: the read takes the node's own
  value/label, not a subtree-concatenated textContent — the m29
  record's honest gap, inherited unchanged.
- An `all()`-family reader for the bounded rects (`all_rects()`-shaped)
  stays out of scope — the wire rows carry rects (M30) and no suite
  has asked; the read family grows one named gap at a time.
