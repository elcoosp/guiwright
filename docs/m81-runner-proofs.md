# M81 — The runner-proof close-out: flaky retries and shard partitioning as wire facts, nextest compatibility proven (§1, §5.2, §11, §16)

**Status:** implemented. The last open halves of the §1 "Test runner
integration" promise are closed with **proofs, not new surface**: the two
e2e facts the M5 record asserted but never demonstrated over a live socket
(retries eventually pass a flaky scenario; shard runs partition the suite)
land in `crates/guiwright-cli/tests/runner_e2e.rs`, and the nextest
compatibility claim is discharged by running `cargo nextest run` against
the adapter integration suites. **No wire change, no CLI flag change, no
test vocabulary** — this milestone moves evidence, not rows.

---

## 1. Why now

After M80 the repo carried a fully-specified M5 runner (specs, workers,
retries with flaky classification, deterministic sharding, JUnit/JSON/HTML
reports) whose §3 testing posture promised "retries → `flaky`
classification" among the e2e-covered facts. The e2e file proved the
*failure* half of retries (`retries_rerun_failed_scenarios_and_count_attempts`
counts three attempts on a permanently failing scenario) and the *list-mode*
half of sharding (`list_mode_needs_no_live_app_and_is_shard_aware`), but two
claims still rested on unit-level evidence only:

1. **"Retries eventually pass a flaky test"** — nothing over a real socket
   showed a scenario that fails once and passes on the retry, landing
   `ScenarioStatus::Flaky` with `attempts == 2` and a green exit (the M5
   exit-code contract: flaky does not fail the run).
2. **"Shard runs partition the suite"** — the interleaved split was proven
   on the filter function and in list mode; a pair of *executed* runs
   (`1/2` + `2/2`) covering exactly the discovered scenarios, each green
   against one app, was not on file.

The same gap sat on the process side: the workspace was **claimed**
nextest-friendly (no hardcoded UDS paths, no shared singletons — every
socket lives in a per-call leaked `tempfile::tempdir()`, every embedded
adapter test owns its app locally) but that claim had never been
discharged by an actual `cargo nextest run` exit 0.

## 2. The proofs

* **Flaky retry over the wire** —
  `retries_eventually_pass_a_flaky_scenario` (`runner_e2e.rs`). A seam
  registered on the server (`flaky_gate`) rejects its first call and
  accepts afterwards; the gate counter is an `Arc<AtomicUsize>` captured in
  the registry closure, so it survives the fresh adapter each attempt's
  fresh connection spawns (the mock only needs `with_state` attached for
  `run_in` to execute the task). The spec carries `retries = 1` and one
  scenario whose single step is `mock call flaky_gate` — the exact M4/M5
  design: the REPL command language is the spec's step language, the mock
  domain (§14) is the fixture hook, no protocol row is touched. The
  assertion pins all three contract points: status `Flaky`, `attempts == 2`,
  `failed == false`, plus the gate seeing exactly two calls.
* **Shard partition over the wire** — `shard_runs_partition_the_suite_exactly`
  (`runner_e2e.rs`). Two executed runs (`1/2`, `2/2`) against one spawned
  app: every record `Passed`, `report.shard` rides each run's report, the
  union is exactly the four discovered scenarios, and the
  `file::name` set has no overlap.
* **Nextest compatibility** — `cargo nextest run` (default features)
  against `guiwright-adapter-egui` and `guiwright-adapter-masonry` (the §11
  M0-framework pair) exits green: nextest's process-per-test model finds
  nothing to collide — no test binds a fixed port or socket path, and the
  runner e2e's leaked tempdirs give every scenario server a unique
  `UDS` path per process. The same structure holds for the CLI's own
  `runner_e2e`/`repl_e2e` suites (each `#[tokio::test]` spawns its own
  server on its own socket).

## 3. Testing posture

The milestone adds two tests and one record. The new e2e tests ride the
existing mock-adapter + real-socket contract pattern; they intentionally
reuse `spawn_server` / `leak_dir` / `uds_opts` helpers so the file's shape
stays uniform. The nextest runs are command-line evidence recorded in the
delivery report, not CI wiring — the §11 story keeps nextest as the local
runner while CI stays on `cargo test`.

## 4. Deferred (recorded)

- HTML report artifact deep links (trace/screenshot embedding) — unchanged
  from M5 §4: the runner does not own per-scenario artifact emission, so
  there is nothing honest to link yet.
- `cargo nextest run` as a CI job — the green run is local evidence; CI's
  matrix keeps `cargo test --workspace` until the runner suites graduate.
- The freya/slint/floem/gpui harness stacks under nextest — the two
  GPU-less M0 adapters carry the proof; the heavy framework graphs stay on
  the CI feature jobs (§17).

## 5. Conclusion

M81 closes the evidence gap between what the M5 runner promised and what
its tests demonstrated: flaky retries and shard partitioning are now wire
facts over a real UDS session, and the nextest-compatibility claim is a
measured exit code instead of a code-reading inference — with the protocol,
the CLI flag surface and the test vocabulary byte-for-byte unchanged.
