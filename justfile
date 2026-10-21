# List available commands
default:
    @just --list

# Install the cargo tools the recipes below use
install-tools:
    cargo install cargo-nextest cargo-llvm-cov cargo-machete cargo-mutants cargo-deny typos-cli zizmor --locked
    rustup component add llvm-tools-preview

# Install the pinned toolchain and the moon-managed git hooks
setup:
    proto install
    moon sync vcs-hooks

# Build the workspace. One `cargo --workspace` invocation, not `moon run :build`: per-crate cargo processes all
# contend for the same target/ lock and serialize.
build:
    cargo build --workspace --locked

build-release:
    cargo build --workspace --release --locked

# Run tests with nextest, plus doctests, which nextest does not run
test:
    cargo nextest run --workspace --all-features --locked
    cargo test --doc --workspace --all-features --locked

# Line coverage; with a threshold, fails below it
coverage threshold="":
    #!/usr/bin/env bash
    set -euo pipefail
    cargo llvm-cov nextest --workspace --all-features --locked --lcov --output-path lcov.info
    cargo llvm-cov report --summary-only
    if [[ -n "{{ threshold }}" ]]; then
      cargo llvm-cov report --summary-only --fail-under-lines "{{ threshold }}"
    fi

# Coverage report, opened in the browser
coverage-html:
    cargo llvm-cov nextest --workspace --all-features --locked --html --open

# Format check and clippy, with and without default features
lint:
    cargo fmt --all --check
    cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
    cargo clippy --workspace --all-targets --no-default-features --locked -- -D warnings

# Clippy on the crates moon reports as affected, in one cargo invocation
lint-affected:
    #!/usr/bin/env bash
    set -euo pipefail
    projects=$(moon query projects --affected | jq -r '.projects[].id')
    if [ -z "$projects" ]; then exit 0; fi
    args=()
    for p in $projects; do args+=(-p "$p"); done
    cargo clippy "${args[@]}" --all-targets --all-features --locked -- -D warnings

fmt:
    cargo fmt --all

# Documentation build, with warnings (including broken intra-doc links) as errors
doc-check:
    RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --workspace --all-features --locked

# Licenses, advisories, bans and sources
deny:
    cargo deny check

# Unused dependencies
machete:
    cargo machete

typos:
    typos

# Workflow audit for permission, credential and supply-chain issues
zizmor:
    zizmor --offline --strict-collection .

# Mutation testing. Slow, so on demand only, not part of `just ci`.
mutants:
    cargo mutants --workspace

# Public API SemVer check against the last published version. Not in `just ci` until a first release exists.
check-api:
    cargo semver-checks check-release

clean:
    moon clean 2>/dev/null || true
    cargo clean

# Run by the moon-managed pre-commit hook
pre-commit:
    cargo fmt --all --check
    typos

# Run by the moon-managed pre-push hook
pre-push: pre-commit lint-affected

# Everything the CI jobs run
ci: lint test doc-check deny machete typos zizmor (coverage "90")
