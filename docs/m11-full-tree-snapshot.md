# M11 — Tree dump: the §7 full-tree snapshot, at last

**Status:** implemented (this milestone lands the tree-dump domain:
`Snapshot::Full` → `TreeDump` — the whole logical tree as typed rows,
depth-first per window root — plus the REPL's `snapshot full`, the
TypeScript `treeDump()`, byte-pinned goldens, a budget row and docs).

---

## 1. Why the dump, and why now

The M0-era `TreeSnapshot` request has carried its own promissory note in
`schema.rs` since the first commit: *"summary in M0; full snapshot rides
the blob side-channel once traces land."* Traces landed in M1; the full
snapshot never followed — the M0 summary (`tree_seq` + `node_count`) is
what `TreeSnapshot` still answers, and the REPL's `snapshot` still prints
a count. Every other read surface (resolve, codegen facts, overlay
badges) reads the tree piecemeal; nothing returns the whole shape.

The consumer case is the CDP "DOM snapshot" analog: CI logs and REPL
sessions want the tree as data — what nodes exist, their roles, names,
values, bounds and nesting — after a failed scenario, before an attach,
or while debugging a locator that "should" match. The REPL is the
day-to-day reader (§16 "REPL / debugging — over GWP"); the TypeScript
client is the programmatic one.

| Hook | What it names | Status before M11 |
|---|---|---|
| §7 tree domain | `TreeSnapshot { window, filter } → TreeSnapshotV1 { seq, nodes, focus }` | summary-only since M0 |
| `schema.rs` M0 comment | "full snapshot … once traces land" | the note this milestone cashes |
| §16 v0.4 multi-window | one target per window | the dump walks every root |
| REPL `snapshot` | "tree @N: M nodes" | a count, not a shape |

## 2. Design

### 2.1 Typed rows, depth-first per window — not an opaque blob

The M0 note said "blob side-channel". Decision: **`TreeDump` carries
typed `SnapshotNodeV1` rows** (key / role / name / value / bounds /
children) as postcard in the response frame, not an opaque compressed
byte string. Three reasons:

1. **Both languages read it natively.** A blob payload forces the
   TypeScript client to ship an LZ4 decoder (the zero-dependency client
   would grow its first dependency) for a debugging payload.
2. **Rows are handles.** Each row's `key` is the same `NodeKeyV1`
   vocabulary `Action` / `Inspect` / `Highlight` take — a dump row feeds
   directly into a follow-up request, no client-side re-resolve.
3. **Bounded honestly.** The debug payload rides the frame with the
   `MAX_FRAME_BYTES` guard; the LZ4 stage stays with trace-frame traffic
   (§15.6), where it was sized to earn its cost. At the §15.1 scale
   (1 k nodes, ~200 B/node) a dump is ~200 KB — well inside the guard.

The hierarchy rides as **depth-first order + a `children` count per
row** — no parent pointers, no per-node depth: a client rebuilds the tree
with a running stack (the REPL's renderer is 20 lines). The walk is
**one depth-first pass per window root** (§16 v0.4): grafted subtrees are
disjoint by construction, roots come in adapter-reported window order,
and a graft-free app is one `main` walk. Session-wide by design — the
connection's resolve scope narrows locators, not whole-tree reads.

### 2.2 Fresh reads, honest facts

`Session::tree_dump` pumps first (the same consistency rule `resolve`
follows), so the dump describes the tree as it is *now* — a scripted diff
that landed mid-session shows up on the next dump (pinned by the
late-change test). Rows carry the §13 fact vocabulary the overlay badges
and codegen already read (`format!("{role:?}")` names, labels, values,
physical-px bounds) plus the focus node, so a REPL dump shows selection
state too.

## 3. Tests + bench

- `guiwright-driver/tests/tree_dump.rs` — the fixture's 8 nodes in exact
  DFS order with exact children counts; the Save row directly usable as
  an `Action` handle; the email input's value in the dump; the
  multi-window adapter's dump covering both window targets; a late rename
  visible on the next dump.
- `guiwright-cli` REPL — `snapshot full` renders the tree indented by the
  rebuilt hierarchy (`Window "Test App" (focus)` → `Group` → six
  children), value rows included; the summary form is byte-unchanged.
- TypeScript — `treeDump()`, the `SnapshotNodeV1`/`TreeDump` shapes,
  golden-pinned `snapshot_full` request (discriminant 14) and `tree_dump`
  reply (discriminant 16, two-row dump), vendored-schema re-pin.
- `guiwright-bench` — `tree_dump_walk` sample (pump + per-window DFS +
  row build over the 1k-node tree; measured p50 ≈ 130 µs on the dev
  host; the serialize cost stays the existing `snapshot_serialize` row) +
  the `tree_dump_walk` row in both `budgets.toml` copies.
- Goldens + published schema: the `snapshot` domain table,
  `Snapshot`/`TreeDump` in the request/response tables, the
  append-only frontier moved to 14/16, bless flow re-run.

## 4. Remaining gaps

- Filtered dumps (`Snapshot::Filtered { selector }`, subtree dumps) land
  behind new `SnapshotV1` variants — the enum is append-only and `Full`
  never reshapes.
- No diff form: a client wanting "what changed" diffs two dumps; a
  server-side diff (the §15.2 one-diff-many-consumers shape) is a later
  milestone if a consumer shows up.
- The REPL prints text; a `--json` output mode for machine consumers
  rides the CLI's output-formatting milestones.

[`Session::tree_dump`]: ../../crates/guiwright-driver/src/session.rs
[`TreeDump`]: ../../crates/guiwright-proto/src/schema.rs
