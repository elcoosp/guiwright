# M13 — Filtered tree dumps: `Snapshot::Filtered`

**Status:** implemented (this milestone lands the §7 tree domain's
scoped form: `Snapshot::Filtered { selector, timeout_ms }` →
`TreeDump` for one subtree, with the REPL's `snapshot under <get-args…>`,
the TypeScript `treeDumpUnder()`, byte-pinned goldens, a budget row,
tests and docs).

---

## 1. Why the filtered form, and why now

The m11 remaining-gaps note named it verbatim: *"Filtered dumps
(`Snapshot::Filtered { selector }`, subtree dumps) land behind new
`SnapshotV1` variants — the enum is append-only and `Full` never
reshapes."* The consumer case is the debugging follow-up: a full dump
tells you what the tree looks like; the natural next question is "what
does **this** subtree look like" — the panel that should contain your
button, the dialog that just opened, the row a locator "should" match.
Dumping 1k nodes to read 8 of them buries the answer.

| Hook | What it names | Status before M13 |
|---|---|---|
| m11 §4 | "filtered dumps land behind new variants" | the note this milestone cashes |
| §8.2 resolve | the auto-wait every read surface shares | `Resolve`/`Screenshot::Element` used it; dumps did not |
| §16 v0.4 | per-connection window scope | honored by resolve, unexercised by dumps |

## 2. Design

### 2.1 Resolve once, walk once — the same rules as every read

`SnapshotV1::Filtered { selector, timeout_ms }` (discriminant 1) resolves
through the **same** `resolve_result` the `Resolve` and
`Screenshot::Element` arms run: auto-wait `Attached` within
`timeout_ms`, honoring the connection's window scope (§16 v0.4), a dead
selector expiring as `Timeout` and a non-matching one failing as
`StrictModeViolation` at timeout 0 — the exact error surface locators
have. The dump is [`Session::tree_dump_under`]: the depth-first walk
rooted at the match, the same row vocabulary and `children` counts as
`Full`, the root row first.

### 2.2 Focus is reported only when it lives inside

`Full` always reports the session focus (it is always inside the whole
tree). A subtree dump reports the focus **only when the focused node is
inside the dumped subtree** — a node outside it is not this dump's fact
to report, and a client rebuilding the hierarchy would otherwise render
a focus marker for a row it never received. Both cases are pinned: the
fixture's window-root focus stays out of a Group dump; a scripted focus
move into the dumped Dialog shows up tied to the actionable row.

## 3. Tests + bench

- `guiwright-driver/tests/tree_dump.rs` — the Group subtree (7 of the
  fixture's 8 rows, exact DFS order + counts, no window root); the
  auto-wait on a late `Group "Dialog"` (resolve inside the dump, focus
  inside reported); typed rejections (`StrictModeViolation` at timeout
  0, `Timeout` with a wait); the connection window scope honored by the
  resolve.
- `guiwright-cli` — `snapshot under <get-args…>` renders the subtree
  with the same indentation rules as `snapshot full` (pinned e2e, plus
  the runner-wording rejection on a miss).
- TypeScript — `treeDumpUnder(selector, timeoutMs?)`, the
  `snapshot_filtered` request vector, vendored-schema re-pin; the
  cross-language e2e dumps the demo app's group subtree over TCP.
- `guiwright-bench` — `tree_dump_filtered` (strict resolve + subtree
  walk + row build over the 1k tree; measured p50 ≈ 125 µs on the dev
  host — the `tree_dump_walk` row stays the full-tree bound) + the row
  in both `budgets.toml` copies.

## 4. Remaining gaps

- Multiple/subset dumps (several selectors, or "everything except") ride
  further `SnapshotV1` variants — the enum stays append-only.
- The REPL prints text; a `--json` output mode for machine consumers
  rides the CLI's output-formatting milestones (the m11 note stands).

[`Session::tree_dump_under`]: ../../crates/guiwright-driver/src/session.rs
[`SnapshotV1`]: ../../crates/guiwright-proto/src/schema.rs
