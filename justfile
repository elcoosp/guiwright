# Release-prep orchestrator — the local mirror of .github/workflows/release.yml.
# Proves a tag is publishable BEFORE it is pushed: crates.io metadata
# strictness, the topo-sorted publish dry-run, and the git-cliff changelog.
# Run `just release-prep` (bump + changelog + Rust dry-run + TS pack
# dry-run) or each recipe alone.

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
# kit's path-dep version req on guiwright-macros + the TypeScript
# client's package.json, the §16 lockstep rule), then refresh both
# lockfiles so --locked publish dry-runs and `npm ci` stay honest.
bump new_version:
    #!/bin/sh
    set -eu
    sed -i "s/^version = \".*\"/version = \"{{new_version}}\"/" Cargo.toml
    sed -i "s/version = \"{{version}}\" }/version = \"{{new_version}}\" }/g" Cargo.toml
    sed -i "s/guiwright-macros = { path = \"..\/guiwright-macros\", version = \"{{version}}\" }/guiwright-macros = { path = \"..\/guiwright-macros\", version = \"{{new_version}}\" }/" crates/guiwright-kit/Cargo.toml
    sed -i "s/^  \"version\": \".*\"/  \"version\": \"{{new_version}}\"/" clients/typescript/package.json
    grep -q "\"version\": \"{{new_version}}\"" clients/typescript/package.json || {
        echo "ts client version sync failed: clients/typescript/package.json"
        exit 1
    }
    cargo metadata --format-version 1 > /dev/null
    # npm ci requires the lockfile to agree with package.json — refresh
    # it when npm exists (a Rust-only release host tolerates its absence;
    # the CI release job runs npm ci and fails loudly on a stale lock).
    if command -v npm >/dev/null 2>&1; then
        (cd clients/typescript && npm install --package-lock-only --silent)
    else
        echo "warning: npm not found — clients/typescript/package-lock.json not refreshed"
    fi
    echo "bumped: {{version}} -> {{new_version}} (rust + ts client lockstep, lockfiles refreshed)"

# Regenerate CHANGELOG.md from the Conventional Commits history, cutting
# the current workspace version as the release section (cliff.toml — the
# curated 1.0.0 journey block rides the template, so regeneration keeps
# it). Robust on a dirty tree: git-cliff walks COMMITTED history, so
# uncommitted milestone work never silently lands in the section — the
# exact manual step for the release manager is printed instead.
changelog:
    #!/bin/sh
    set -eu
    if ! command -v git-cliff >/dev/null 2>&1; then
        echo "git-cliff is required (CI pins git-cliff@2.14.2): cargo install git-cliff --locked"
        exit 1
    fi
    if [ -n "$(git status --porcelain)" ]; then
        echo "changelog: the working tree is dirty — the section renders from COMMITTED history only."
        echo "  Manual step: commit the milestone work (or curate the section by hand against"
        echo "  the cliff.toml 1.0.0 journey block) BEFORE tagging; the CI changelog refresh"
        echo "  runs on the pushed tag with full history."
    fi
    git-cliff --tag "v{{version}}" --output CHANGELOG.md
    if ! grep -qm1 '^## \[`{{version}}`\]' CHANGELOG.md; then
        echo "changelog: no '{{version}}' section was cut (check the tag and the commit range)"
        exit 1
    fi
    grep -m1 -A1 '^## \[`{{version}}`\]' CHANGELOG.md

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

# The full local pre-tag gate — one atomic, idempotent command: metadata
# strictness, the changelog cut, the topo-sorted publish dry-run (all 18
# crates, verification ON) and the TypeScript client's build + publish
# dry-run, ending in a READY FOR TAG verdict. Atomicity: the bump (when a
# new version is requested) and the changelog cut are the only steps that
# write — every file they touch is snapshotted first and restored if any
# step fails, so a failed run exits non-zero with the tree exactly as it
# found it (no half-bumped manifests, no half-cut section). Idempotent:
# re-running with the same version regenerates byte-identical outputs and
# exits 0.
release-prep new_version="":
    #!/bin/sh
    set -eu
    work=$(mktemp -d)
    restore() {
        cp "$work/Cargo.toml" Cargo.toml
        cp "$work/kit.toml" crates/guiwright-kit/Cargo.toml
        cp "$work/package.json" clients/typescript/package.json
        cp "$work/package-lock.json" clients/typescript/package-lock.json
        cp "$work/CHANGELOG.md" CHANGELOG.md
        rm -rf "$work"
        echo "release-prep: FAILED — every written file restored, the tree is as it was"
    }
    cp Cargo.toml "$work/Cargo.toml"
    cp crates/guiwright-kit/Cargo.toml "$work/kit.toml"
    cp clients/typescript/package.json "$work/package.json"
    cp clients/typescript/package-lock.json "$work/package-lock.json"
    cp CHANGELOG.md "$work/CHANGELOG.md"
    if ! (
        set -eu
        if [ -n "{{new_version}}" ]; then
            just bump "{{new_version}}"
        fi
        just check-metadata
        just changelog
        just publish-dry-run
        # The TypeScript client's gate (the release.yml mirror): the
        # build-check, then the pack dry-run proving the tarball npm
        # publish ships (the same proof the CI release job runs).
        (cd clients/typescript && npm run build && npm publish --dry-run --access public)
    ); then
        restore
        exit 1
    fi
    rm -rf "$work"
    version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
    echo "READY FOR TAG: v${version} — changelog cut, 18 crates publish-verified, TS client pack-verified"
