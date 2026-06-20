set shell := ["sh", "-cu"]

# rtk-cli wraps commands to reduce terminal noise.
# Rust toolchain is pinned via rust-toolchain.toml (v1.96).

alias c := check
alias t := test

# ---- Formatting -----------------------------------------------------------

fmt:
    rtk cargo fmt

# Verify formatting without modifying files (CI-friendly).
fmt-check:
    rtk cargo fmt -- --check

# ---- Build / check --------------------------------------------------------

check:
    rtk cargo check

build:
    rtk cargo build

build-release:
    rtk cargo build --release

clippy:
    rtk cargo clippy --all-targets --all-features

# Treat clippy warnings as failures (CI-friendly).
clippy-deny:
    rtk cargo clippy --all-targets --all-features -- -D warnings

# ---- Tests ----------------------------------------------------------------

test:
    rtk cargo test

# ---- Dependency hygiene ---------------------------------------------------

machete:
    rtk cargo machete

# ---- Maintenance ----------------------------------------------------------

clean:
    rtk cargo clean

# Update dependencies and refresh the lockfile.
update:
    rtk cargo update

# Install lefthook git hooks.
hooks-install:
    rtk lefthook install

# ---- CI recipes (used by .github/workflows/release.yml) --------------------

ci-fmt-check:
    rtk cargo fmt -- --check

ci-clippy:
    rtk cargo clippy --all-targets --all-features -- -D warnings

ci-test:
    rtk cargo test

ci-build:
    rtk cargo build --release

# Validate pushed tags against Cargo.toml version. Reads `git push` stdin.
ci-check-release-tag-version:
    python scripts/check_release_tag_version.py

# Generate raw release notes via git-cliff, then optionally polish via AI.
# Falls back to the raw notes when Anthropic env is not configured.
ci-release-notes:
    mkdir -p dist
    rtk git-cliff --latest --output dist/release-notes.raw.md
    python scripts/polish_release_notes.py --input dist/release-notes.raw.md --output dist/release-notes.md

# Same as the local `ci` recipe, but expressed via the ci-* primitives so the
# GitHub Actions pipeline and local CI stay perfectly in sync.
ci-check: ci-fmt-check ci-clippy ci-test

# ---- Groups ---------------------------------------------------------------

# Fast local loop: format + check + clippy.
dev: fmt check clippy

# Full pipeline run the same way CI would: verify formatting, type-check,
# lint-deny, test, and check for unused dependencies.
ci: fmt-check check clippy-deny test machete

# ---- Run the binary -------------------------------------------------------

run *args:
    rtk cargo run -- {{ args }}

config *args:
    rtk cargo run -- config {{ args }}

doctor *args:
    rtk cargo run -- doctor {{ args }}

list *args:
    rtk cargo run -- list {{ args }}

audit *args:
    rtk cargo run -- audit size {{ args }}
