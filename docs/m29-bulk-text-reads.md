# M29 — The bulk text read (`all_text_contents`)

**Status:** implemented. One kit method over an existing session seam,
one budget row, one consumer example — no new session API, no waiter, no
protocol change (reads are client-side by design, the m26 record's
standing note).

---

## 1. Why now

The M26 record closed its locator-all work with an honest gap: "bulk
*content* reads (`all_text_contents()`-shaped helpers) are not part of
this milestone — the visitor can collect per-item reads today; a
dedicated bulk reader lands when a suite asks for it." The visitor shape
works but pays per-item costs: one pump per visited item, one borrowed
locator at a time, the whole set walked twice (once by `resolve_all`,
once by the pinned re-resolves). A suite that only wants the data —
compare a rendered list against expected labels, feed a report — needs
one walk and zero pumps. This milestone adds exactly that reader.

## 2. Design: the set read mapped to data

The load-bearing decision: **the read is the `all()` enumeration's
snapshot mapped to data, not a new resolution path.** The same
non-strict `resolve_all` walk `count()` and `all()` take produces the
match set; each match contributes the text the text locators match —
`label` first, then `value` (the `HasText` check's source order, §8.3
"AccessKit exposes text through labels and values"). That buys:

* **Composability.** Every `get_by_text(t)` match implies `t` appears in
  the corresponding read — the reader and the locators cannot disagree
  about what an element's text is, because they read the same
  consumer-node accessors.
* **Positional stability.** A node with neither label nor value reads as
  an empty string, keeping `Vec` positions aligned with the document
  order the set read established (a `Vec<String>` maps 1:1 onto the
  `all()` visitor order).
* **Set semantics for free.** Hidden nodes are included, the window pin
  scopes the walk, narrowing scopes the read — the `all()`/`count()`
  rules, unchanged. A late-arriving node joins the next read (snapshot
  reads do not wait; `wait_for` or an assertion covers that).

The read records nothing: the `count()` rule — resolution without an
action is not a replayable step — extends to content reads (no codegen
line, no trace action; the trace's tree snapshots carry the same facts).
No session method was needed: `Session::resolve_all` and the tree's
label/value accessors were already public, so the read stays a kit-side
composition and the m26 record's wire note ("the enumeration would ride
the same `Resolve`-reply extension") remains the one open protocol
question.

## 3. Surface

- `crates/guiwright-kit/src/app.rs` — `Locator::all_text_contents` (the
  bulk reader, with the snapshot rules in the rustdocs).

## 4. Tests

- `crates/guiwright-kit/tests/all_text_contents.rs` — end-to-end on the
  mock adapter: document order over the button fixture (hidden included),
  the empty set, `.first()` narrowing, the label-first/then-value rule,
  the window pin scoping the read (the two-"Save" discriminator fixture),
  the late-arriving button joining the next read (rendezvous-channel
  deterministic), and the read-records-nothing codegen rule.
- `crates/guiwright-kit/examples/all_text_contents.rs` — the consumer
  example: the whole set, the narrowed set, the label/value rule, the
  window-scoped read, and the codegen nothing-recorded rule printed.
- `bench/budgets.toml` + `crates/guiwright-bench/src/budgets.toml` —
  the `text_contents` row (sample: one `all_text_contents()` over the
  button fixture — the `resolve_all` set read plus four label/value
  reads; the set shape mirrors the `locator_all` row without the
  per-item pumps), enforced by the CI guard.

## 5. Remaining gaps (honest list)

- No wire domain / REPL row — the read rides `resolve_all` client-side
  by design; a remote bulk read would extend the `Resolve` reply exactly
  as the m26/m22 records name (append-only, byte-pinned, one golden
  pass) and stays open until a remote suite asks.
- `all_values()` (a value-first bulk read for form dumps) is a one-line
  variation of the same walk — deliberately not added until a suite asks
  for it, to keep the surface to the named gap.
- Multi-line / descendant text runs: the read takes the node's own
  label/value, not a subtree-concatenated textContent — AccessKit
  carries text on text-bearing nodes, and a subtree walk would duplicate
  what the tree dump already provides (`snapshot full`).
