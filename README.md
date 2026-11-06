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

This repository is at **M0** (architecture `docs/architecture.md`):

- [x] `guiwright-core` — tree service, selector engine, actionability,
      zero-polling auto-wait, expect/assertions
- [x] `guiwright-proto` — GWP wire schema (`V1`), length-prefixed
      postcard framing, LZ4 blob side channel, UDS transport
- [x] `guiwright-driver` — session loop, mock adapter, GWP-over-UDS server
- [x] `guiwright-adapter-egui` — embedded adapter, frame digests, input batching
- [x] `guiwright-adapter-masonry` — embedded adapter over `RenderRoot`
- [x] `guiwright-bench` — criterion suites + CI perf-budget guard
      (`bench/budgets.toml`, §15.1)

## Workspace

| Crate | Role |
|---|---|
| `guiwright-core` | Selector resolution, tree diffing, wait/expect — substrate only |
| `guiwright-proto` | GWP protocol: schema, framing, blobs, UDS |
| `guiwright-driver` | Sessions, adapters trait, UDS server |
| `guiwright-adapter-egui` | egui: AccessKit tree + paint-digest determinism |
| `guiwright-adapter-masonry` | Masonry/Xilem: `RenderRoot` headless |
| `guiwright-bench` | Criterion benches + `check_budgets` perf guard |

## Quick start

```bash
cargo test --workspace          # full suite
cargo clippy --workspace --all-targets -- -D warnings
cargo bench -p guiwright-bench  # criterion suites
cargo run --release -p guiwright-bench --bin check_budgets   # perf guard (§15.7)
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
