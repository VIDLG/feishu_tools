set shell := ["sh", "-cu"]

# fst is a pure-Rust CLI. pixi is used as a unified toolchain manager so the
# exact same set of CI/release tooling (just / git-cliff / lefthook / rtk-cli)
# is reproducible across local dev and GitHub Actions. Rust itself is still
# pinned via rust-toolchain.toml (v1.96).
#
# Git Bash/MSYS injects a pseudo environment variable named `!::` on Windows.
# pixi's activation environment capture trips over it, so strip it before every
# `pixi run` invocation. Outside Windows this is a harmless no-op.
pixi := if os_family() == "windows" { "env -u '!::' pixi" } else { "pixi" }

alias c := check
alias t := test

# ---- Formatting -----------------------------------------------------------

fmt:
    {{ pixi }} run -- rtk cargo fmt

# Verify formatting without modifying files (CI-friendly).
fmt-check:
    {{ pixi }} run -- rtk cargo fmt -- --check

# ---- Build / check --------------------------------------------------------

check:
    {{ pixi }} run -- rtk cargo check

build:
    {{ pixi }} run -- rtk cargo build

build-release:
    {{ pixi }} run -- rtk cargo build --release

clippy:
    {{ pixi }} run -- rtk cargo clippy --all-targets --all-features

# Treat clippy warnings as failures (CI-friendly).
clippy-deny:
    {{ pixi }} run -- rtk cargo clippy --all-targets --all-features -- -D warnings

# ---- Tests ----------------------------------------------------------------

test:
    {{ pixi }} run -- rtk cargo test

# ---- Dependency hygiene ---------------------------------------------------

machete:
    {{ pixi }} run -- rtk cargo machete

# ---- Maintenance ----------------------------------------------------------

clean:
    {{ pixi }} run -- rtk cargo clean

# Update dependencies and refresh the lockfile.
update:
    {{ pixi }} run -- rtk cargo update

# Install lefthook git hooks.
hooks-install:
    {{ pixi }} run -- rtk lefthook install

# ---- CI recipes (used by .github/workflows/release.yml) --------------------

ci-fmt-check:
    {{ pixi }} run -- rtk cargo fmt -- --check

ci-clippy:
    {{ pixi }} run -- rtk cargo clippy --all-targets --all-features -- -D warnings

ci-test:
    {{ pixi }} run -- rtk cargo test

ci-build:
    {{ pixi }} run -- rtk cargo build --release

# Validate pushed tags against Cargo.toml version. Reads `git push` stdin.
ci-check-release-tag-version:
    {{ pixi }} run -- python scripts/check_release_tag_version.py

# Generate raw release notes via git-cliff, then optionally polish via AI.
# Falls back to the raw notes when Anthropic env is not configured.
ci-release-notes:
    mkdir -p dist
    {{ pixi }} run -- rtk git-cliff --latest --output dist/release-notes.raw.md
    {{ pixi }} run -- python scripts/polish_release_notes.py --input dist/release-notes.raw.md --output dist/release-notes.md

# Same pipeline the GitHub Actions release job runs.
ci-check: ci-fmt-check ci-clippy ci-test

# ---- Groups ---------------------------------------------------------------

# Fast local loop: format + check + clippy.
dev: fmt check clippy

# Full pipeline run the same way CI would: verify formatting, type-check,
# lint-deny, test, and check for unused dependencies.
ci: fmt-check check clippy-deny test machete

# ---- Run the binary -------------------------------------------------------

run *args:
    {{ pixi }} run -- cargo run -- {{ args }}

config *args:
    {{ pixi }} run -- cargo run -- config {{ args }}

doctor *args:
    {{ pixi }} run -- cargo run -- doctor {{ args }}

list *args:
    {{ pixi }} run -- cargo run -- list {{ args }}

audit *args:
    {{ pixi }} run -- cargo run -- audit size {{ args }}
