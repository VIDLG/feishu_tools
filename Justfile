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
