# Release-prep orchestrator — the local mirror of .github/workflows/release.yml.
# Proves a tag is publishable BEFORE it is pushed: crates.io metadata
# strictness, the topo-sorted publish dry-run, and the git-cliff changelog.
# Run `just release-prep` (bump + changelog + dry-run) or each recipe alone.

default:
    @just --list

# The workspace version, read from the root manifest (single source of
# truth — every member inherits it, [workspace.package] version).
version := `sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1`

# The publish order, verified against `cargo metadata` (the dependency
# graph at m79): core/macros/shot are the internal roots; kit and bench
# close the graph. The CI workflow's publish-dry-run step carries the
# same list — keep both in sync.
topo := "guiwright-core guiwright-macros guiwright-shot guiwright-inspect guiwright-proto guiwright-trace guiwright-driver guiwright-adapter-egui guiwright-adapter-floem guiwright-adapter-freya guiwright-adapter-gpui guiwright-adapter-iced guiwright-adapter-masonry guiwright-adapter-slint guiwright-viewer guiwright-cli guiwright-kit guiwright-bench"

# Strict crates.io metadata gate: every crate carries description,
# license, repository, keywords and categories (direct or workspace-
# inherited). The workspace.package table defines the four shared
# fields — description stays per-crate by design (each crate's own).
check-metadata:
    #!/bin/sh
    set -eu
    fail=0
    for field in license repository keywords categories; do
        grep -qE "^${field} *=" Cargo.toml || { echo "MISSING workspace.package ${field}"; fail=1; }
    done
    for f in crates/*/Cargo.toml; do
        crate=$(sed -n 's/^name = "\(.*\)"/\1/p' "$f" | head -1)
        for field in description license repository keywords categories; do
            grep -qE "^${field}(\.workspace)? *=" "$f" || { echo "MISSING ${field}: ${crate}"; fail=1; }
        done
    done
    if [ "$fail" -ne 0 ]; then
        echo "metadata gate: FAILED"
        exit 1
    fi
    echo "metadata gate: all 18 crates carry description/license/repository/keywords/categories"

# Bump the workspace version and every internal version reference in
# one pass ([workspace.package] + the workspace.dependencies table +
# kit's path-dep version req on guiwright-macros), then refresh the
# lockfile's member entries so --locked publish dry-runs stay honest.
bump new_version:
    #!/bin/sh
    set -eu
    sed -i "s/^version = \".*\"/version = \"{{new_version}}\"/" Cargo.toml
    sed -i "s/version = \"{{version}}\" }/version = \"{{new_version}}\" }/g" Cargo.toml
    sed -i "s/guiwright-macros = { path = \"..\/guiwright-macros\", version = \"{{version}}\" }/guiwright-macros = { path = \"..\/guiwright-macros\", version = \"{{new_version}}\" }/" crates/guiwright-kit/Cargo.toml
    cargo metadata --format-version 1 > /dev/null
    echo "bumped: {{version}} -> {{new_version}} (lockfile refreshed)"

# Regenerate CHANGELOG.md from the Conventional Commits history, cutting
# the current workspace version as the release section (cliff.toml).
changelog:
    git-cliff --tag "v{{version}}" --output CHANGELOG.md
    grep -m1 -A1 "^## {{version}}" CHANGELOG.md

# The publishability proof: cargo publish --dry-run across the whole
# workspace — verification ON. The packaged family re-resolves against
# the LOCAL members, so this proves packaging, metadata and the version
# graph for all 18 crates in dependency order (core → macros → shot →
# inspect → proto → trace → driver → adapters → viewer → cli → kit →
# bench) before any tag is pushed. Per-crate dry-runs against the
# crates.io index only resolve once the rc.1 family's first publish
# lands; --allow-dirty because release-prep runs on the working tree
# BEFORE any commit/tag — the CI loop runs on the pushed (clean) tag.
publish-dry-run:
    #!/bin/sh
    set -eu
    cargo publish --workspace --dry-run --allow-dirty --locked
    echo "publish dry-run: all 18 workspace crates packaged and verified"

# The full local pre-tag gate: bump (optional — empty keeps the current
# version), metadata strictness, changelog cut, topo-sorted dry-run.
release-prep new_version="":
    #!/bin/sh
    set -eu
    if [ -n "{{new_version}}" ]; then
        just bump "{{new_version}}"
    fi
    just check-metadata
    just changelog
    just publish-dry-run
