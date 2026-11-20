# M81 — The own runner drives a real adapter end-to-end (§1, §16 v0.5)

**Status:** implemented (glue/gpui gates fully green including the bench
target; the runner e2e suite green over the real egui adapter). This
milestone closes the two gaps the m80 record left open: the residual
clippy-pedantic gate blockers in the `glue/gpui` workspace, and the
runner's e2e story having only ever ridden the framework-free
`MockAdapter`. **No wire change**: this milestone moves test reality,
not protocol rows.

---

## 1. Why now

Two debts were recorded against m80. First, the m80 record's own status
line: "gpui green on tests/example with two residual clippy-pedantic
errors in the glue workspace to fix in a follow-up" — the follow-up is
M1 here, and the gate-red set was concrete: an unused `index` binding in
`GpuiAdapter::act`, a wildcard match arm in the embedded test that
silently matches only `NodeKey::Logical`, and — the real one — the m80
bench target calling `GpuiAdapter::introspect_once()`, a method that
existed only on the floem side, so `cargo clippy --all-targets` failed
to compile the bench at all (the "green on tests/example" row never
covered `--benches`). Second, the §1 promise ("Test runner integration
…, per-test app isolation") and the §16 v0.5 own-runner row had no
evidence against a real framework: every runner e2e drove
`MockAdapter::with_fixture`.

## 2. M1 — the glue/gpui gate blockers

* **The bench seam** — `GpuiAdapter` gains
  `pub fn introspect_once(&mut self) -> u64` (one full fold: the real
  scene draw digested over the framework-recorded geometry, the boot
  fold's unit), mirroring the floem harness's public measurement seam
  and its "public so the m80 bench can measure exactly one fold"
  contract. The bench compiles again and `--benches` is a gate that
  runs.
* **The dead index** — `act` computed `let (index, bounds)` and read
  only the bounds; the binding drops to `let (_, bounds)`.
* **The wildcard arm** — `tests/gpui_embedded.rs` matched
  `other => panic!(...)` where exactly one variant remained; the arm is
  now `other @ guiwright_core::NodeKey::Logical { .. }`, so a future
  `NodeKey` variant fails loud instead of hiding in the panic.
* **Pedantic posture** — the glue's `[lints]` table stays the sanctioned
  carve-out surface (`cast_possible_truncation`, `cast_sign_loss`,
  `missing_errors_doc` are explicit allows there); no `#[allow]` landed
  in source. The stricter-than-CI probe (`-W clippy::pedantic` on the
  command line) re-fires only the table's own carve-outs.

## 3. M2 — the runner e2e over the real egui adapter

* **The app is real and Guiwright-free** — the egui counter+form app
  (stock widgets: two buttons, a `TextEdit`, two labels) owns its state
  inside the app closure. The factory closure captures nothing, which
  is the whole `Send` story: `spawn_with_seams` requires only
  `F: FnOnce() -> A + Send`, while the `!Send` adapter stays on the
  session thread.
* **Why egui** — the §4.1 adapter is windowless and GPU-optional (a
  GPU-less host caches the probe failure and answers the pixel rows
  `Unsupported`), so the suite runs in any CI without a display or a
  vulkan driver. The tree, input, actions and clock paths are pure egui.
* **The suite is data** — a plain `gwtest/v1` spec: `get role button
  Increment` → `click` → `assert text text "Count" is "Count: 2"`, then
  a fill scenario that observes the count the first scenario left
  behind — real app state carried across the fresh connection each
  attempt opens (the §14 connection-per-attempt isolation). The
  assertion's locator pins the label's node family because egui
  renders each label as a `Label`/`TextRun` twin pair whose tree order
  rides content hashes; the expectation then waits for the real
  content — the two-stage m54 read, order-proof by construction.
* **Per-run app isolation** — the same suite runs green against a
  second, freshly booted app: the first scenario's `Count: 2`
  expectation only holds when each app boots isolated (the §11
  "one app per shard" story, proven at suite scale).
* **The binary is the contract** — the same suite then runs as a
  process: `guiwright test <dir> --uds <sock> --report json`, exit
  code `0`, and the `gwrun/v1` report on disk with
  `"passed": 2, "failed": 0` — the §1 "cargo test" experience,
  validated end-to-end.
* **Honest row** — egui's `run_in` stays `Unsupported` (the trait
  default), so the §14 `mock reset` state-seam path is not part of this
  suite; isolation rides the connection-per-attempt seam and the
  fresh-app proof instead.

## 4. Example

`crates/guiwright-adapter-egui/examples/counter_form.rs` joins the
walkthrough family (iced m76, freya m79, the glue examples): boot the
§4.1 adapter over the stock app, resolve by role+name, click and fill
through the semantic pipeline, print what the tree reflects after every
step.

```text
cargo run -p guiwright-adapter-egui --example counter_form
```

## 5. Evidence

* `cargo clippy --manifest-path glue/gpui/Cargo.toml --all-targets
  --locked -- -D warnings` — exit 0, zero warnings.
* `cargo test --manifest-path glue/gpui/Cargo.toml --locked` — 7 passed,
  0 failed (the embedded suite drives the real gpui counter+form).
* `cargo test --manifest-path glue/gpui/Cargo.toml --locked --benches` —
  the `gpui_introspection` criterion entry runs.
* `cargo test -p guiwright-cli --test cli_runner_e2e` — 3 passed
  (runner-to-green over the real app, fresh-app isolation, binary exit
  code + report).
* `cargo run -p guiwright-adapter-egui --example counter_form` — the
  walkthrough prints each step green.
