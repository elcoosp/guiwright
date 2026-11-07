# M5 — the own runner: `guiwright test` (§5.2, §11, §14, §16)

**Status:** implemented (this milestone extends `crates/guiwright-cli` with the
runner: `spec`, `runner`, `report` modules and the `guiwright test`
subcommand).
**Scope:** §16 v0.5 "Test runner, fixtures, parallel/shard, retries, reports —
v0.5 (own runner)" plus the §5.2 `guiwright-cli` crate row's remaining
halves ("nextest/report glue, sharding") and the §11 session-sharding story
("multiple apps per host").
**Evidence base:** `docs/architecture.md` §5.2 (crate table), §11 (CI
parallelism/sharding), §14 (reset seams = fixture parity), §16 (parity map
v0.5 row); the M4 deferred list (`docs/m4-cli-repl.md` §4) names this row
first.

---

## 1. Why the runner is the next milestone

After M4 every interactive entry point exists (REPL, show-trace, bless,
install-deps, remote codegen), but the §16 parity map still has exactly one
open version-tagged row: the **v0.5 own runner**. The M4 deferral records it
verbatim — "`guiwright test` — nextest/report glue and the §16 'v0.5 own
runner' row." The wire surface it needs has existed since M2/M3 (GWP over
UDS **and** TCP, resolve/action with auto-wait, clock, mock seams); what is
missing is the CI-side loop that turns recorded interactions into
**repeatable, sharded, retried, reported** runs:

| Architecture hook | What it names | Status before M5 |
|---|---|---|
| §16 parity map | "Test runner, fixtures, parallel/shard, retries, reports — v0.5 (own runner)" | open (MVP = nextest-native, done in M1) |
| §5.2 crate table | `guiwright-cli` — "nextest/report glue, sharding" | no runner, no reports, no sharding |
| §11 | "Guiwright adds session-level sharding over the protocol (multiple apps per host)" | no shard split anywhere |
| §14 | "reset_state(scope) fixture parity" (Playwright contexts) | wire-only; no runner drives it between scenarios |
| §1 mission | "JUnit/HTML reports" | none |

Multi-window (§16 v0.4, protocol revision `V1` append-only), the
`guiwright codegen <app-cmd>` launcher wrapper (§13 overlay session) and
live viewer streaming (§12 "live mode") stay deferred — recorded in M4 §4,
unchanged by this milestone.

## 2. Design: the spec is the REPL script

The own runner's test **definition format is data, not another e2e language**
(§18's dilution risk): a `gwtest/v1` spec is a small TOML file whose steps
are strings in the **M4 REPL command language**. The interactive debugger
and the CI runner share one command core — a session rehearsed in the REPL
pastes into a spec verbatim, and every step maps 1:1 onto an existing GWP
request (**no protocol change**; the M2 conclusion carries over a second
time).

```toml
# tests/guiwright/login.spec.toml — gwtest/v1
target = "uds:///tmp/app.gwp"   # optional: file default target (§5.3 form)
timeout_ms = 5_000              # optional: resolve/action deadline for every scenario
retries = 1                     # optional: re-run failed scenarios (default: 0)

[[scenario]]
name = "save the document"      # unique within the file (strict)
steps = [
  "get role button Save",       # the M4 REPL locator surface, quoted freely
  "click",                      # applies to the resolved node at a frame boundary
  "expect role text \"Saved\"",   # assertion wording; fails the scenario on timeout
]
```

Rules (enforced by the parser, `deny_unknown_fields` strictness):

- A spec is `target` / `timeout_ms` / `retries` plus one or more
  `[[scenario]]` tables; a scenario is `name` + `steps` (≥ 1) with optional
  per-scenario `timeout_ms` / `retries` overrides.
- `bye` / `exit` / `quit` are rejected in steps — the runner owns the
  session lifecycle (connect → handshake → steps → `Bye` per attempt).
- Resolution order: CLI `--uds` / `--tcp` override the spec `target`;
  without either the run is a config error (exit 2). A spec without a
  target and no CLI target is an error naming the file.

### 2.1 Semantics

- **One connection per attempt.** Every scenario attempt opens a fresh GWP
  connection, performs the §7 handshake, feeds each step through the same
  `Repl::step` core, sends `Bye`. A step whose outcome carries an error
  line fails the scenario with that message; the remaining steps do not
  run.
- **`expect` joins the REPL core** (client-side only, no wire change): it
  resolves like `get`, stores the current node, and words failures as
  expectations ("expectation failed: …") so runner reports read as
  assertions.
- **Retries.** A failed attempt is retried up to `retries` times. Final
  status: `passed` (first attempt), `flaky` (passed on a later attempt),
  `failed`. The attempt count rides the reports.
- **Fixtures / isolation = §14 reset seams.** One driver session serves
  every connection (the M0 server model — verified in
  `driver/src/server.rs`), so scenarios that mutate app state reset it
  through the mock domain: a `mock reset <scope>` step is the runner's
  fixture hook, the exact §14 "fixture parity with Playwright contexts"
  sentence. Process-per-test isolation remains the kit + nextest story
  (§11); the runner adds the **session-level** half.
- **Sharding.** `--shard k/n` (1-indexed) splits the *discovered,
  deterministically sorted* scenario list by interleaved index
  (`i % n == k-1`). Each CI job pairs its shard with its own app instance
  via `--uds` / `--tcp` — §11's "multiple apps per host", and the
  honest parallelism story: one driver session serializes all connections,
  so N workers against one socket share one app, while N apps per host
  scale linearly.
- **Workers.** `--workers N` (default 1) runs N tasks pulling from the
  shared queue; each still owns one connection per attempt. Reports are
  emitted in discovery order regardless of completion order.

### 2.2 Reports (§16 "JUnit/HTML", §5.2 "report glue")

`--report list` (default), `json`, `junit`, `html` — repeatable. File
reports land under `--out <dir>` (default `guiwright-report/`):
`report.json` (`gwrun/v1` schema: run metadata, shard, summary, per-scenario
records with attempts/duration/error), `junit.xml` (one `<testsuite>` per
spec file, `<failure message>` per failed scenario, flaky = passed
testcase), `report.html` (self-contained summary + table, the
`trace.html` precedent). stdout keeps the live progress lines
(`PASS` / `FAIL` / `FLAKY` / `SKIP` + summary); `--list` prints the
discovered scenario ids and exits 0 (shard-aware).

Exit codes: `0` all passed (flaky included), `1` any scenario failed,
`2` usage/config error (bad flags, unreadable spec, duplicate scenario
names, no scenarios found, missing target).

## 3. Testing posture

The M4 pattern repeats — the framework-free mock adapter + a real socket is
the contract test. TDD: each module's tests landed before its
implementation.

- `crates/guiwright-cli/tests/runner_e2e.rs` — discovery + full pass run
  over UDS (asserting the wire actions landed: the mock adapter records
  them), a failing scenario (exit 1, failure message in stdout and JUnit),
  retries → `flaky` classification, `--list` (+ shard filtering),
  `--shard` split determinism, workers > 1, spec-level and CLI-level
  target override, strict spec errors (duplicate scenario, empty steps,
  unknown key, `bye` in steps, missing target).
- `src/spec.rs` unit tests — parsing, strictness, override resolution.
- `src/report.rs` unit tests — `gwrun/v1` shape, JUnit structure and XML
  escaping, HTML content and escaping, list rendering.
- `src/args.rs` unit tests — the `test` subcommand flag surface.

## 4. Deferred (recorded)

- Multi-window / tabs (§16 v0.4) — unchanged, needs the `V1` append-only
  protocol revision and its own spike doc.
- `guiwright codegen <app-cmd>` — the app-launching wrapper lands with the
  overlay session (§13 highlight handles).
- Live trace-viewer streaming (§12 "live mode") — unchanged.
- HTML per-scenario trace deep links — the report names the artifact dir
  convention (`target/guiwright/<run>/…`), clickable links wait for the
  runner to own trace emission (the kit already records traces; the
  declarative runner drives the wire directly).

## 5. Conclusion

M5 closes the last version-tagged row of the §16 parity map below v1: the
same GWP wire that serves the REPL, the TypeScript client and the kit now
serves a declarative CI runner with Playwright's run-shape vocabulary —
specs, list mode, retries with flaky detection, deterministic sharding over
multiple apps per host, and JUnit/JSON/HTML reports — with no protocol
change and no new external dependencies beyond `toml`/`serde` (already
workspace pins).
