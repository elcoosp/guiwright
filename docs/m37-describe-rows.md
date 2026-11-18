# M37 — The describe REPL rows (`describe` + the enriched `hit` row)

**Status:** implemented. Two client rows over the M36 `Describe` domain —
the m32/m36 records' named REPL consumer ("the REPL renders the hit as
the handle (`accesskit#2`) + rect — role/name would ride a
node-describe domain"): a `describe <get-args…>` command (one node's
identity facts) and the `hit` row rendering the role/name beside the
handle. No wire change (the domain, its golden vectors and the TS
`describe()` shipped as M36), no session change, no budget row (the
REPL rows are client-side rendering over the already-budgeted
`node_describe` round trip — a second row would measure the same
conversation twice, the m35 precedent).

---

## 1. Why now

M36 shipped the describe read's protocol half and deliberately left the
REPL out — the m30→m35 pattern repeated: the domain first, the REPL
rows as their own pass. The debugging loop the records kept naming
closed one step in: `hit` answered *which handle* but rendered
`accesskit#2`; `get` knew the selector's describe but a hit has no
selector; and the REPL could not answer "what IS this node?" for a
handle it had just minted without a whole-subtree dump. This milestone
is that named pass: the REPL joins the M36 domain's client row without
touching the protocol.

## 2. Design: reads, and one enrichment

* **`describe <get-args…>` is a read, not a resolve-store** (the m35
  rule, unchanged): `last` and the action vocabulary keep aiming where
  `get`/`hit` aimed. The locator resolves strict-mode first — the same
  vocabulary every locator command speaks — with the M35 read-pass's
  **snapshot semantics** (`timeout_ms: 0`, no arrival wait): a
  debugging read answers now; an absent match is the immediate
  strict-mode error, never a partial row. The handle then answers the
  facts: role, accessible name, value, bounds, generation.
* **The value prints.** `TextInput "Email" value "ada@example.dev"` —
  the value is the fact the `value` locator vocabulary matches (§8.1)
  and the question a REPL debugging session asks about an input; the
  `snapshot full` rows already render it, one node does too.
* **The `hit` row enriches, never blocks.** One `Describe` round trip
  behind the hit gives the row its role/name
  (`hit (76, 32) → Button "Save" (accesskit#2) (generation 0) rect=…`).
  An enrichment failure degrades to the bare handle form — the hit
  itself is the read, and the facts ride one extra round trip; a node
  removed between the two reads is the honest case (the hit happened,
  the facts are gone). The handle stays in the row either way — it is
  the action vocabulary (`last`, the actions aim at it), and the
  click-what-you-see loop keeps working against a driver that never
  heard of `Describe`.
* **Rendering is the established vocabulary.** The dump-row fact shapes
  (`Button "Save"`, the quoted name, the `@`-rect turned into the
  `get`-row rect form), the handle form (`accesskit#2`), the generation
  — nothing new to learn, everything composable with the existing rows.

## 3. Surface

- `crates/guiwright-cli/src/repl.rs` — the `describe` dispatch arm +
  `describe_command` (strict resolve with the snapshot timeout, one
  `Describe` round trip, the read-not-store rule in the rustdocs), the
  `describe_facts` enrichment behind `hit_command` (graceful fallback
  documented), the `render_described` row renderer, the command-list
  row.

## 4. Tests

- `crates/guiwright-cli/tests/repl_e2e.rs` — `repl_describe_read` over
  a real UDS socket: the identity-facts row (Save), the value form
  (Email), the read-not-store rule (`get` → `describe` → `last` still
  aims the get's node), the immediate strict-mode error on a ghost, the
  help row; `repl_hit_test_read` re-pins the enriched hit row
  (`Button "Save" (accesskit#2)`, rect and generation intact, `last`
  and `click` unchanged).
- Red→green: the tests failed against the unfixed REPL (unknown-command
  error for `describe`, the bare-handle hit row), passed 14/14 after
  the two rows landed.

## 5. Remaining gaps (honest list)

- No `all`-row enrichment: enumerating a set's facts would open one
  `Describe` round trip per row (the m35 batch note, unchanged — `all`
  stays the identity + generation + rect rows, and `snapshot under`
  answers whole-set content questions).
- No facts beyond the identity four (role/name/value/bounds): state
  flags ride the dump rows and the assertion vocabulary (the m36
  record's standing note, unchanged here).
- The enrichment degrades silently to the handle form when the driver
  predates M36 — the REPL speaks to a same-version driver in practice;
  a future protocol-negotiation row would surface it (no suite has
  asked).
