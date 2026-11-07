<div align="center">

# guiwright

**Playwright-grade test & automation driver for Rust GUI frameworks.**

One driver, every kit: locate, act on, assert and inspect widgets through
the AccessKit accessibility tree — embedded in-process or over a wire
protocol — with auto-wait semantics, frame-boundary discipline and
enforced performance budgets.

[![CI](https://github.com/elcoosp/guiwright/actions/workflows/ci.yml/badge.svg)](https://github.com/elcoosp/guiwright/actions/workflows/ci.yml)
[![Changelog](https://img.shields.io/badge/changelog-git--cliff-8A2BE2)](CHANGELOG.md)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue)](#license)

</div>

## Status

This repository is at **M2** (architecture `docs/architecture.md`):

- [x] `guiwright-core` — tree service, selector engine, actionability,
      zero-polling auto-wait, expect/assertions
- [x] `guiwright-proto` — GWP wire schema (`V1`), length-prefixed
      postcard framing, LZ4 blob side channel, UDS transport
- [x] `guiwright-driver` — session loop, mock adapter, GWP-over-UDS server
- [x] `guiwright-adapter-egui` — embedded adapter, frame digests, input batching
- [x] `guiwright-adapter-masonry` — embedded adapter over `RenderRoot`
- [x] `guiwright-bench` — criterion suites + CI perf-budget guard
      (`bench/budgets.toml`, §15.1)
- [x] `guiwright-shot` — screenshot codec, YIQ diff, baselines + bless flow (§10)
- [x] `guiwright-trace` — `gwtrace/v1` zip recorder, SPSC ring → writer thread (§12)
- [x] `guiwright-adapter-iced` — Level-2 engine (logical tree, role registry,
      action routing); harness glue behind the `iced` feature (§4.6)
- [x] `guiwright-adapter-freya` — Level-1 support (id bridge, key sequences
      with KeyUp, frame-quantized event buffer); glue behind the `freya` feature (§4.3)
- [x] `guiwright-kit` + `guiwright-macros` — embedded App, Locators, `#[test]`,
      traces, screenshots, nextest-aware artifact naming (§5.2, §6.3, §11)
- [x] `guiwright-viewer` — `gwtrace/v1` archive reader, timeline model
      (tree resolved at any frame from snapshots + diffs), self-contained
      `trace.html` emitter (§12); `App::export_trace_viewer`
- [x] `guiwright-inspect` — codegen recorder with the minimal-stable-selector
      cost model (TestId > role+name > label > text) and the overlay plan
      (§13); egui painter + masonry draw-ops in the adapters
- [x] `guiwright-adapter-slint` — testing-backend introspection engine,
      `(component, item)` id bridge, `mock_elapsed_time` clock plan,
      testing build helper; the glue runs downstream (slint's skia lock
      chain conflicts with freya's — recorded in the crate docs, §4.4)
- [x] Clock service — frame-sized fast-forward stepping, pause/resume
      semantics, `clock.bin` ledger, full kit `app.clock()` surface (§14)
- [x] Mock domain — state seams (`#[guiwright_kit::seam]` +
      `register_seams!`), `run_in` runner-context execution, canned
      overrides, `app.mock()` surface, GWP `Mock` routes (§14)
- [x] Remote-mode GPUI spike — clock + mock domains end-to-end over a real
      UDS socket; findings in `docs/m2-gpui-remote-spike.md` (§17)

## Workspace

| Crate | Role |
|---|---|
| `guiwright-core` | Selector resolution, tree diffing, wait/expect — substrate only |
| `guiwright-proto` | GWP protocol: schema, framing, blobs, UDS |
| `guiwright-driver` | Sessions, adapters trait, UDS server |
| `guiwright-adapter-egui` | egui: AccessKit tree + paint-digest determinism |
| `guiwright-adapter-masonry` | Masonry/Xilem: `RenderRoot` headless |
| `guiwright-bench` | Criterion benches + `check_budgets` perf guard |
| `guiwright-shot` | PNG codec, weighted squared YIQ diff, baseline store + bless |
| `guiwright-trace` | `gwtrace/v1` zip recorder (bounded ring, hash-deduped frames) |
| `guiwright-adapter-iced` | iced Level-2 engine; `iced` feature gates the harness glue |
| `guiwright-adapter-freya` | freya Level-1 support; `freya` feature gates the runner glue |
| `guiwright-adapter-slint` | slint testing-backend engine (framework-free); glue contract in the crate docs |
| `guiwright-kit` | Embedded App, Locators, expect, clock, mock seams, codegen, traces |
| `guiwright-macros` | `#[guiwright_kit::test]` + `#[guiwright_kit::seam]` proc macros |
| `guiwright-viewer` | Archive reader, timeline resolver, self-contained `trace.html` |
| `guiwright-inspect` | Codegen recorder + selector cost model + overlay plan |

## Quick start

```bash
cargo test --workspace          # full suite
cargo clippy --workspace --all-targets -- -D warnings
cargo bench -p guiwright-bench  # criterion suites
cargo run --release -p guiwright-bench --bin check_budgets   # perf guard (§15.7)
```

### Writing a test (embedded mode, §6.3)

```rust
use accesskit::Role;
use guiwright_kit::{expect, App};

#[guiwright_kit::test]
fn login_flow(app: &mut App) -> Result<(), String> {
    app.get_by_role(Role::Button, "Save").click().map_err(|e| e.to_string())?;
    expect(app.get_by_text("Welcome, Ada")).to_be_visible().map_err(|e| e.to_string())?;
    // GUIWRIGHT_BLESS=1 creates baselines; failing runs emit .new.png/.diff.png
    Ok(())
}
```

```bash
GW_TRACE=1 cargo nextest run          # per-test gwtrace zip archives (§12)
GUIWRIGHT_BLESS=1 cargo nextest run   # bless failing screenshot baselines (§10)
```

### Virtual clock, state seams and codegen (M2)

```rust
use guiwright_kit::{register_seams, App};

#[guiwright_kit::seam]
fn reset_db(state: &mut AppState) -> Result<(), AppError> {
    app_state::reset_to_fixture(state)
}

#[guiwright_kit::test]
fn checkout_flow(app: &mut App) -> Result<(), String> {
    app.register_seams(&register_seams![reset_db]);
    app.mock().reset_state("seed-42").map_err(|e| e.to_string())?;
    app.clock().install_virtual().map_err(|e| e.to_string())?;
    app.clock().fast_forward(Duration::from_secs(30)).map_err(|e| e.to_string())?;
    app.enable_codegen();                       // §13 interaction → kit code
    app.get_by_role(Role::Button, "Save").click().map_err(|e| e.to_string())?;
    let (code, warnings) = app.codegen_finish().unwrap();
    Ok(())
}
```

The perf guard fails the build on budget regressions; targets live in
[`bench/budgets.toml`](bench/budgets.toml) and CI applies a documented ×3
headroom.

## Changelog & releases

Changelogs are generated from [Conventional
Commits](https://www.conventionalcommits.org) by [git-cliff](https://git-cliff.org)
(`cliff.toml`); see [`CHANGELOG.md`](CHANGELOG.md). Tagging `vX.Y.Z` triggers
the release workflow, which regenerates the changelog, packages the crates
and publishes a GitHub release.

## License

Dual-licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE-2.0](LICENSE-APACHE-2.0))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.
