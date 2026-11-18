# M40 — The REPL bulk-read rows (`texts` + `values`)

**Status:** implemented. Two REPL commands over the existing M39
`DescribeAll` domain — the m35 record's named follow-up ("No REPL
`text`/`value` bulk reads (the m29/m31 kit readers' REPL half): … a
`texts <get-args…>` row lands when a debugging session asks for it —
one more client-side rendering over the same domain"). `texts
<get-args…>` answers what the set *says*, `values <get-args…>` what the
set *carries*. No wire change (the domain, its golden vectors and the
TypeScript `describeAll()` shipped as M39), no session change, no
budget row (the m35 rule, restated: the rows are client-side rendering
over the already-budgeted `describe_all_wire` round trip — a second row
would measure the same conversation twice).

---

## 1. Why now

M35 gave the REPL the set reads (`count`/`all`) but honestly left the
content question to the tree dump: "the dump answers content questions
in the REPL today." The objection was cost — enumerating a set's facts
meant one `Describe` round trip per row (the m35 batch note). M39
retired that objection: the `DescribeAll` walk already carries the
name and the value beside every row's identity, so the m29/m31 kit
readers' REPL half costs exactly what `all` costs. The debugging
session the m35 note waited for is the one M39 built the road for —
*what does everything this selector matches actually say / carry*,
answered in one round trip, no dump indirection.

## 2. Design: the kit readers' source orders, the REPL's row shape

* **`texts` reads label-first** (the m29 source order): the accessible
  name, falling back to the value — the read the text locators match
  (the `HasText` check's source order).
* **`values` reads value-first** (the m31 source order): what the form
  *carries*, falling back to the name — the `getByValue` source order.
  The fixture's Email input answers `"Email"` to `texts` and
  `"ada@example.dev"` to `values`: the two reads can never silently
  agree on the wrong datum, the distinction the m31 record pinned on
  the kit, now at the interactive surface.
* **Rows are the handle form + the quoted content**
  (`accesskit#5 "Email"`) — the handle keeps every row directly usable
  as the action vocabulary's target (the dump-row rule, M11), the
  quoting is the dump rows' content form (`render_described`'s
  `{value:?}`).
* **The m35 read rules, unchanged.** Both commands are reads, never
  resolve-stores: `last` and the actions keep aiming where
  `get`/`hit` aimed. Strict mode aside (the non-strict walk, hidden
  and disabled included — the m30 set semantics), an empty set is one
  info line — never an error (the snapshot-read rule, one set instead
  of one point).
* **The empty-string rule keeps rows 1:1.** A node with neither name
  nor value renders `""` — positions stay 1:1 with the document order
  the set read established (the m31 rule), so the Nth row is always
  the Nth match.
* **The fallback is the m35 batch note, honestly kept.** A driver that
  predates M39 cannot answer the content in one round trip, so the
  read degrades to the `ResolveAll` walk plus one `Describe` per row —
  the exact cost the m35 record named when it left the rows out. A row
  whose `Describe` fails (the node left between the reads — the m37
  honest case) renders `""` rather than dropping the row.

## 3. Surface

- `crates/guiwright-cli/src/repl.rs` — the `texts`/`values` dispatch
  arms, `texts_command`/`values_command` + the shared
  `bulk_content_command` (one `DescribeAll` round trip, the
  read-not-store rule in the rustdocs), the `describe_row` per-row
  fallback fetch, `render_bulk_row` (the handle + quoted-content form),
  and the two command-list rows.

## 4. Tests

- `crates/guiwright-cli/tests/repl_e2e.rs` —
  `repl_bulk_read_rows_texts_and_values` over a real UDS socket: the
  source orders on the base fixture's Email input (`"Email"` vs
  `"ada@example.dev"`), the four-button walk in document order (Save
  first, node 2; hidden + disabled included), the empty set's info
  line, the read-not-store rule (`get` → `texts` → `last` still aims
  the get's node), and the help listing both rows — plus
  `repl_bulk_read_rows_source_orders_on_labelled_inputs` on the M28
  extension band (three labelled inputs; the two source orders
  disagree on every row, positions 1:1 with the walk).
- Red→green: both suites failed against the unfixed REPL
  (`unknown command 'texts'` — the m35 record's red shape), passed
  16/16 after the commands landed.

## 5. Remaining gaps (honest list)

- No TypeScript row: the client reads already answer content at set
  width through `describeAll()`; the client-side bulk reads are the
  M41 record's rows (shipped alongside this pass, one milestone each
  surface).
- The REPL fallback path (pre-M39 driver) is exercised by design
  review only — the suite always speaks to a same-version driver; the
  same honest note the m37/m39 records carry for their enrichments.
- No per-row scope surface: the reads inherit the connection's window
  scope (the M30/M6 discipline) — `window use` + `texts` is the scoped
  form, the pass adds no scope vocabulary of its own.
