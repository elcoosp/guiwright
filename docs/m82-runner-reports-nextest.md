# M82 — The artifact deep links: the runner's reports link what its steps wrote, and the nextest story hardens (§1, §5.2, §10, §16)

**Status:** implemented. The one m81 deferral with a user-visible surface
closes — the HTML report deep links each scenario's artifact paths and the
`gwrun/v1` JSON carries them as a per-scenario `artifacts` array — plus two
nextest-hardening fixes the compatibility audit surfaced. **No wire change,
no CLI flag change, no test vocabulary**: the runner never emits artifacts
itself, it reads the paths its own steps already wrote back out of the
`shot:` / `video:` Ok lines the M4 REPL language answers today.

---

## 1. Why now

M81 recorded three deferrals. The first — HTML report artifact deep links —
stayed open because "the runner does not own per-scenario artifact
emission, so there is nothing honest to link yet." That framing was half
right: the runner does not own emission, but it never needed to. Since M5
every scenario's steps run through the same [`Repl`] core the interactive
debugger uses, and two of its commands already write files and answer with
the path:

- `screenshot <path>` (§10 capture) → `shot: <path> (<w>×<h>, frame <n>)`
- `video start` / `video stop` (§10) → `video: <path> (<n> frames, …)`

The Ok lines carry the exact path the REPL just wrote. What was missing was
a runner that *kept* them: `run_attempt` skimmed the step outcomes for
`Err` lines only and dropped every other fact on the floor. The other two
deferrals (nextest as a CI job; the heavy glue stacks under nextest) stay
deferred — this milestone instead hardens the *claim* m81 already proved
green: the audit pass over every test binary found two latent collisions
fixed here.

## 2. The artifact seam (runner reads, never writes)

* **Harvest** — `artifact_from_line` (`crates/guiwright-cli/src/runner.rs`)
  recognizes the two file-carrying Ok shapes above and nothing else
  (`video: recording started` harvests nothing — no `(` trailer, no path).
  `run_attempt` returns `(Result<(), String>, Vec<String>)`: the first
  `Err` line still fails the attempt, but artifacts written by steps before
  the failure ride home. `run_with_retries` accumulates across every
  attempt, failures included — a scenario that screenshotted on attempt 1
  and failed on attempt 3 shows the file.
* **JSON** — each `gwrun/v1` scenario object gains `"artifacts": [...]`,
  always emitted (the `error` field's established always-present style), so
  consumers can read `scenarios[i].artifacts` without optional-shape
  handling. Additive only: the m81-pinned fields are byte-identical.
* **HTML** — the report table gains an `artifacts` column; every path
  renders as `<a href="…">…</a>` with the same HTML escaping every other
  cell gets (the escape test pins a `<weird>` path renders inert). The
  report stays self-contained: it links the files the steps wrote, it does
  not inline megabytes of PNG bytes into a table.
* **JUnit** — unchanged. `JUnit` has no artifact convention that CI
  consumers actually render; the `<failure>` message stays the machine
  surface (the m5 decision, unchanged).

## 3. Nextest hardening (the audit's two finds)

* **Golden bless under nextest** —
  `crates/guiwright-proto/tests/common/mod.rs` adds
  `bless_write_permitted(bless, nextest)`: the `GUIWRIGHT_BLESS=1` write to
  the fixed repo-path goldens (`tests/schema-golden.json`,
  `clients/typescript/test/golden.json`) is refused when `NEXTEST` is set —
  nextest's per-test process fan-out would race the write across every
  leaked env var. Both golden tests fall through to the
  compare-against-committed branch, which passes; the documented bless flow
  stays plain `cargo test` (the doc comments now say so).
* **No `/`-rooted literals in test specs** —
  `cli_target_overrides_the_spec_target` pointed its connect-failure spec
  at `uds:///nonexistent-gw-runner-test.sock`, a root-level path literal.
  It now joins a leaked temp dir, so the connect-failure path exercises
  nothing outside the test's own tree (the house style every other suite
  already follows).
* **The glue lockfile rides the rc.1 bump** — the m81 release bump
  refreshed the main workspace lockfile but left
  `glue/slint/Cargo.lock`'s seven path-dep member entries at their
  pre-bump `0.4.0` (floem/gpui were bumped correctly). Current cargo
  refuses to resolve the glue workspace under `--locked` — its CI job's
  exact invocation. The lock now carries `1.0.0-rc.1` member versions
  (member entries only, verified by diff).

## 4. Testing posture

* `report_files_are_written_for_json_junit_and_html` grows the mixed
  suite a `screenshot <path>` scenario (the `spawn_shot_server` helper
  scripts pixels via `with_frame_pixels`, the `wire_screenshot` contract)
  and pins all three surfaces: the record carries the path, the JSON
  `artifacts` array matches, the HTML links it, and the file exists on
  disk. The JUnit counts assertion moves to `tests="3"`.
* Unit tests: `artifact_from_line` (both shapes, status lines, degenerate
  strings), artifacts accumulating across a flaky retry pair, escaped
  artifact links in the HTML.
* `bless_writes_only_outside_nextest` pins the guard's truth table (both
  golden binaries compile the shared helper).
* The nextest proof stays command-line evidence: `cargo nextest run` green
  over `guiwright-cli` (the runner/repl e2e suites), `guiwright-adapter-egui`,
  `guiwright-adapter-masonry` and `guiwright-adapter-iced`.

## 5. Deferred (recorded)

- `cargo nextest run` as a CI job — CI's matrix keeps `cargo test`;
  the green runs remain local evidence (m81's posture, unchanged).
- The freya/slint/floem/gpui glue stacks under nextest — the GPU-less
  adapter pair plus iced carry the proof; the glue workspaces stay on
  their CI feature jobs (§17).

## 6. Conclusion

M82 closes the artifact half of the §16 report row with the same
"moves evidence, not rows" discipline m81 used for the retry/shard half:
the reports link what the steps wrote, the JSON contract grows one
additive array, the protocol, the CLI flag surface and the test vocabulary
are byte-for-byte unchanged — and the nextest claim survives an audit
that looked for what it had not yet been asked to find.
