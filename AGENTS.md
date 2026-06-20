# AGENTS.md — instructions for AI coding agents working on `fst`

> Project-level guidance. Zed loads this automatically; other agents (Claude
> Code, Cursor, etc.) should read it before making changes.

## TL;DR

`fst` is a small Rust orchestrator around `lark-cli` for Feishu/Lark Drive
storage audits, backups, and conservative delete plans. The codebase is
intentionally compact — favor minimal, surgical changes over big rewrites.

## Toolchain (use it)

- **Always prefer `just` over raw `cargo`**. The `Justfile` wraps every common
  command with `pixi run -- rtk ...` so the environment is reproducible.
- First-time setup: `pixi install && just hooks-install`.
- Rust itself is pinned via `rust-toolchain.toml` (v1.96, edition 2024).

### Common recipes

| Recipe | Purpose |
|---|---|
| `just dev` | Fast local loop: fmt + check + clippy |
| `just ci` | Full CI pipeline: fmt-check + check + clippy-deny + test + machete |
| `just fmt` / `just fmt-check` | Format / verify formatting |
| `just clippy` / `just clippy-deny` | Lint / treat warnings as failures |
| `just test` | Run tests |
| `just check` | `cargo check` only |
| `just run <args>` | Run the binary |
| `just hooks-install` | Install lefthook git hooks |

### Git hooks (lefthook)

- `pre-commit`: runs `just fmt-check`
- `pre-push`: runs `just clippy-deny` + `just test` + tag/version validation

Hooks are already wired via `lefthook.yml`. Don't disable them; if a hook
fails, fix the cause.

## Architecture principles

1. **`lark-cli` is the API backend.** `fst` orchestrates. Do not add direct
   Feishu HTTP calls unless `lark-cli` genuinely cannot do the job.
2. **External Lark JSON → `serde_json::Value`.** Different `lark-cli` commands
   expose slightly different shapes (`token` vs `file_token`, etc.). Keep that
   boundary dynamic; use typed structs only for data we own (config, reports,
   delete plans, results).
3. **Concurrent code lives in `src/concurrency.rs`.** Use the existing
   `run_concurrent` helper with `tokio::sync::Semaphore`. Don't spawn raw
   `tokio::spawn` loops for batch work.
4. **Errors flow through `src/error.rs` (`LarkError`).** Map new failure modes
   to a semantic exit code (see `src/error.rs`'s `ExitCode` mapping). Avoid
   `anyhow::anyhow` in new code; use typed variants.
5. **No global state mutation.** Pass cwd to subprocesses via
   `tokio::process::Command::current_dir`. Do not call `env::set_current_dir`.
6. **Prefer community crates over hand-rolled utilities.** We already use
   `fs-err`, `dunce`, `thiserror`, `tokio-retry`, `humantime`, `syntect-assets`,
   `anstream`, `colorchoice-clap`, `clap-verbosity-flag`. Reach for these
   before writing new helpers.

## Where things live

```
src/
  main.rs              entry + clap CLI
  error.rs             LarkError + ExitCode mapping
  config.rs            TOML config (~/.fst/config.toml)
  lark.rs              lark-cli subprocess wrappers
  concurrency.rs       run_concurrent + retry helpers
  highlight.rs         syntect TOML/JSON syntax highlighting
  report.rs            CSV/JSON report writers (was report/, now flat)
  csvutil.rs           CSV row helpers + field aliases
  util.rs              small cross-cutting helpers
  commands/
    mod.rs             clap subcommand definitions
    audit/             size audit (split: model, common, media, export, size)
    backup.rs          backup export / download-files
    delete.rs          delete plan / apply
    doctor.rs          env/scope diagnostics
    list.rs            drive search / folder listing
    quota.rs           drive quota_details
    report.rs          report summary
```

## Output / data layout

- Config: `~/.fst/config.toml`
- Workspace root (configurable): `~/.fst/storage/`
  - `backups/{exports,files,media}/`
  - `reports/`

Do **not** reintroduce hardcoded paths like `D:/backup/...`. Use config +
`dirs` crate.

## Conventions

- **Conventional commits** — `feat:`, `fix:`, `docs:`, `ci:`, `refactor:`,
  `chore:`, `perf:`, `test:`, `style:`. The changelog (`git-cliff`) and
  release notes pipeline depend on this.
- **Edition 2024.** New code should use 2024 idioms (`let Some(x) = ... else`,
  `use` nesting, etc.) where they read better.
- **Comments**: only for non-obvious intent, constraints, or tradeoffs. Do not
  restate the code.
- **No new dependencies without justification.** If you add one, explain why
  an existing crate cannot do it.

## Releasing

Releases are tag-driven:

1. Bump `version` in `Cargo.toml`.
2. `git commit -am "chore: bump version to X.Y.Z"`
3. `git tag vX.Y.Z`
4. `git push origin main --tags`

The `pre-push` hook validates `Cargo.toml` version vs the pushed tag.
`release.yml` then builds the Windows exe, generates changelog via
`git-cliff`, optionally polishes via Anthropic (when
`ANTHROPIC_API_KEY`/`ANTHROPIC_BASE_URL` secrets are set), and publishes a
GitHub Release.

## Don't

- Don't run `cargo fmt` directly — use `just fmt`.
- Don't bypass git hooks.
- Don't commit `target/`, `.pixi/`, `dist/`, or `.env`.
- Don't introduce global mutable state.
- Don't rewrite Lark JSON parsing into strict typed structs (see principle #2).
- Don't fix unrelated bugs or formatting in a focused PR.
