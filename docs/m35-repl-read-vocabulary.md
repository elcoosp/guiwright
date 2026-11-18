# M35 — The REPL read-vocabulary pass (`count` + `all`)

**Status:** implemented. Two REPL commands over the existing M30
`ResolveAll` domain — the m30 record's named follow-up ("a REPL row
lands with the REPL's own read-vocabulary pass, not as a rider on this
domain"): `count <get-args…>` answers the set size, `all <get-args…>`
enumerates the set rows. No wire change (the domain, its golden vectors
and the TypeScript `resolveAll()` shipped as M30), no session change,
no budget row (the REPL rows are client-side rendering over the
already-budgeted `resolve_all_wire` round trip — a second row would
measure the same conversation twice).

---

## 1. Why now

M30 shipped the set read's protocol half and deliberately left the
REPL out: "`resolve-all` is a client-surface command, and the REPL
speaks command-shaped reads (`get`, `count` does not exist either); a
REPL row lands with the REPL's own read-vocabulary pass." The REPL's
debugging loop could resolve one node (`get`), aim at a point (`hit`)
and act — but could not answer the two questions every debugging
session asks before acting: *how many match?* and *what exactly
matches?* A suite-side answer existed (the kit's `count()`/`all()`,
M22/M26); the interactive surface — the place a human pokes a live app
— had none. This milestone is that named pass: the REPL joins the
M30 domain's client row without touching the protocol.

## 2. Design: reads, not resolve-stores

The load-bearing decision: **`count` and `all` are reads — they never
store the current node.** The REPL's `get`/`hit` resolve-store (the
click-what-you-see loop: resolve, then `last`/actions aim at it) is a
single-node convention; a set read has no honest single pick, and
storing "the first match" would silently re-aim the action vocabulary.
So the reads leave `current` untouched — `get`, `count`, `last` composes
exactly as the tests pin — and a set row that *should* become the
action target goes through `get`/`hit` explicitly.

Everything else is the domain's semantics, rendered, not re-decided:

* **Strict mode aside.** The read rides `ResolveAll` — the non-strict
  walk, hidden and disabled nodes included — the same set semantics the
  kit's `count()` documents (M22).
* **Rows are the established rendering.** `all` prints the identity +
  generation + rect line `last`/`hit` already print (the dump-row
  handle form, directly usable as the `Action` node), document order
  (the `ResolvedAll` ordering contract).
* **Totality.** An empty set is a valid answer everywhere: `count`
  prints `0 matches`, `all` prints one info line — never an error (the
  snapshot-read rule M22 recorded, one set instead of one point).
* **Discoverability.** The command list carries both rows; the m30
  record's "REPL speaks command-shaped reads" framing holds — `count`
  and `all` take the exact `get` locator vocabulary
  (`role | text | label | testid | value`), nothing new to learn.

## 3. Surface

- `crates/guiwright-cli/src/repl.rs` — the `count`/`all` dispatch arms,
  `count_command`/`all_command` (one `ResolveAll` round trip each, the
  read-not-store rule in the rustdocs), and the two command-list rows.

## 4. Tests

- `crates/guiwright-cli/tests/repl_e2e.rs` — `repl_read_vocabulary_count_and_all`
  over a real UDS socket: the fixture's four buttons counted
  (non-strict), the singular and empty `count` forms, the `all` rows in
  document order (Save first, node 2, rect pinned against the hit-test
  row's rendering), the empty set's info line, the read-not-store rule
  (`get` → `count` → `last` still aims the get's node), and the help
  listing both rows.
- Red→green: the test fails against the unfixed REPL (unknown-command
  error), passes 13/13 after the two commands land.

## 5. Remaining gaps (honest list)

- No per-row role/name describe: the `ResolvedMatchV1` row carries the
  node identity, so the REPL prints the handle form (`accesskit#2`) —
  role/name would ride the node-describe domain that does not exist
  yet (the m32 record's standing gap, unchanged here; the dump carries
  role/name for whole-subtree questions).
- No REPL `text`/`value` bulk reads (the m29/m31 kit readers' REPL
  half): the tree dump answers content questions in the REPL today; a
  `texts <get-args…>` row lands when a debugging session asks for it —
  one more client-side rendering over the same domain.
- The reads inherit the connection's window scope (the M30/M6
  discipline) — `window use` + `count` is the scoped form; the pass
  adds no scope surface of its own.
