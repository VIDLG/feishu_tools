# fst

`fst` is a compact Feishu/Lark storage toolkit for listing Drive resources, auditing large storage consumers, exporting backups, and deleting only human-confirmed cleanup targets.

`fst` uses `lark-cli` as the Feishu API backend. Rust handles configuration, orchestration, reporting, sorting, backup layout, and delete-plan safety.

Chinese documentation: [README.zh-CN.md](README.zh-CN.md)

Design notes: [docs/design.md](docs/design.md) / [中文](docs/design.zh-CN.md)

## Workflow

```text
config
  -> doctor
  -> list
  -> audit size
  -> backup
  -> delete plan
  -> delete apply
```

The tool separates diagnosis, backup, and deletion. Reports are CSV/JSON so each step can be reviewed, edited, and resumed manually.

## Run

During development:

```bash
cargo run -- <command>
```

After building/installing:

```bash
fst <command>
```

Examples:

```bash
fst doctor
fst --verbose doctor
fst list --limit 20
fst audit size --mode full --limit 20
```

## Toolchain

`fst` ships with a unified toolchain via [`pixi`](https://pixi.sh) so local dev
and CI use the exact same set of tools (`just`, `git-cliff`, `lefthook`,
`rtk-cli`). Rust itself is pinned by `rust-toolchain.toml` (v1.96, edition 2024).

First-time setup:

```bash
pixi install
just hooks-install   # install lefthook git hooks
```

Common recipes (all routed through `pixi run`):

| Recipe | Purpose |
|---|---|
| `just dev` | Fast loop: fmt + check + clippy |
| `just ci` | Full CI: fmt-check + check + clippy-deny + test + machete |
| `just fmt` / `just fmt-check` | Format / verify formatting |
| `just clippy` / `just clippy-deny` | Lint / warnings-as-errors |
| `just test` | Run tests |
| `just run <args>` | Run the binary |

Prefer `just <recipe>` over raw `cargo <cmd>` — the `Justfile` wraps every
command with `pixi run -- rtk ...` so the environment is reproducible.

## Configuration

Default config path: `~/.fst/config.toml`.

Example config: [src/config.example.toml](src/config.example.toml).

Create it:

```bash
fst config init
```

Show the effective config. `fst config` and `fst config show` are equivalent:

```bash
fst config
```

Use another config file:

```bash
fst --config path/to/config.toml list
```

Default config:

```toml
[storage]
# Local workspace root for all fst outputs. Defaults to ~/.fst/storage.
# root = "~/.fst/storage"

[list]
# lark-cli search page size.
page_size = 20
# Default search keyword. Empty means no keyword filter.
query = ""
# Only list resources owned by you. Shared-in files usually do not count
# against your storage, so this defaults to true.
mine = true

[audit]
# Number of largest rows written to top-* CSV reports.
top_limit = 100
# Include embedded media when auditing/exporting, so size estimates are closer
# to backup/delete decisions.
include_media = true
# Max concurrent lark-cli subprocesses for export/download.
# Set to 1 to reproduce the old serial behavior.
concurrency = 4

[delete]
# Mark known-size rows at or above this size as delete candidates.
min_mb = 100.0
# Keep only the largest N delete candidates. 0 means no cap.
top = 0
# Keep unknown-size rows in the review plan instead of hiding them.
include_unknown = true
```

Output layout is convention-based, not configurable:

```text
~/.fst/storage/
  backups/
    exports/   # Feishu-native docs exported by backup/audit
    files/     # Ordinary Drive files downloaded by backup download-files
    media/     # Embedded document media downloaded by full audit/backup
  reports/     # CSV/JSON reports and delete plans
```

Backups and reports share one workspace so a run can be reviewed and resumed
from the generated CSV files without hunting across directories.

## Doctor

```bash
fst doctor
```

`doctor` performs the broadest practical checks by default, without dangerous writes.

It checks:

- local config paths
- `lark-cli --version`
- `lark-cli auth status`
- common scope hints
- `drive +search` probe
- Drive root `files list` probe
- an auto-selected exportable sample, when available
  - `docs +fetch` for doc/docx samples
  - `drive +export`

Optional explicit document probes:

```bash
fst doctor --doc <doc_url_or_token>
fst doctor --doc-type docx --doc-token <token>
```

`doctor` can find systemic problems, but it cannot prove every file is accessible. Feishu permissions are per-resource and can also be affected by folder permissions, ownership, wiki routing, tenant policy, and secure labels.

## List

### Search-based listing

```bash
fst list
fst list --limit 100
fst list --query "keyword"
fst list --doc-types docx,sheet,bitable,file,folder,wiki,slides
fst list --mine=false
```

Default behavior uses `drive +search --as user`.

Output:

```text
list-<timestamp>.json
list-<timestamp>.csv
```

### Folder tree listing

List Drive root:

```bash
fst list --folder-token ""
```

List a folder:

```bash
fst list --folder-token <folder_token>
```

Recursive listing:

```bash
fst list --folder-token <folder_token> --recursive
fst list --folder-token <folder_token> --recursive --max-items 1000
```

Folder listing uses `drive files list`, not search. The output includes `path` to distinguish duplicate file names.

## Size audit

```bash
fst audit size --mode metadata
fst audit size --mode export
fst audit size --mode full
```

| Mode | Downloads? | Use case |
|---|---:|---|
| `metadata` | No | Fast scan. Reliable mainly for ordinary Drive files when metadata has size. |
| `export` | Yes, export native docs | Rank Feishu native docs by exported file size. |
| `full` | Yes, export + media | Closest to actual backup size. |

Use previous list as input:

```bash
fst audit size --mode metadata --input list-xxx.csv
fst audit size --mode export --input list-xxx.csv
fst audit size --mode full --input list-xxx.csv
```

Outputs:

```text
size-results-metadata-<timestamp>.json
size-results-metadata-<timestamp>.csv
top-large-metadata-<timestamp>.csv

audit-results-<timestamp>.json
audit-results-<timestamp>.csv
top-large-docs-<timestamp>.csv
failed-<timestamp>.json
```

`full` mode attempts to download embedded resources from document markdown tags such as:

```text
source
img
image
file
whiteboard
```

## Backup

### Export Feishu-native resources

```bash
fst backup export
fst backup export --limit 100
fst backup export --input list-xxx.csv
fst backup export --include-media
fst backup export --include-media=false
```

Supported export types:

```text
doc
docx
sheet
bitable
slides
```

`backup export` defaults to including embedded media because backup should preserve resources when possible.

#### Concurrency

Export and download loops run up to `[audit].concurrency` lark-cli
subprocesses at a time (default 4). The serial behavior can be reproduced with
`concurrency = 1`.

#### Resuming interrupted runs

Export and download are idempotent across interruptions. When `--force` is
not set, an existing target file is skipped. If the input CSV carries a size
hint (`metadata_bytes` / `total_bytes` / `export_bytes`), fst additionally
requires the local file size to match it; truncated or partially-written
files from a Ctrl-C are re-downloaded automatically. Files without a size hint
fall back to the existence-only check.

### Download ordinary Drive files

```bash
fst backup download-files --input list-xxx.csv
fst backup download-files --input list-xxx.csv --limit 100
fst backup download-files --input list-xxx.csv --force
```

This downloads rows where `doc_type=file`.

Output directory:

```text
~/.fst/storage/backups/files/
```

Reports:

```text
file-download-results-<timestamp>.json
file-download-results-<timestamp>.csv
```

## Delete plan

Generate a human-reviewable delete plan from audit/list CSV.

```bash
fst delete plan --input top-large-docs-xxx.csv
fst delete plan --input audit-results-xxx.csv --min-mb 500
fst delete plan --input audit-results-xxx.csv --min-mb 500 --opened-before 180d
fst delete plan --input audit-results-xxx.csv --updated-before 2024-01-01
fst delete plan --input list-xxx.csv --include-unknown
fst delete plan --input list-xxx.csv --inspect-wiki
```

Outputs:

```text
delete-plan-<timestamp>.json
delete-plan-<timestamp>.csv
```

Before deletion, manually edit the CSV and set:

```text
delete_candidate = true
human_confirmed = true
```

### Wiki handling

`drive +delete` does not directly delete wiki resources. Use:

```bash
fst delete plan --input list-xxx.csv --inspect-wiki
```

This calls `drive +inspect` and tries to replace wiki rows with underlying Drive object type/token.

## Report summary

Summarize any fst CSV report:

```bash
fst report summary --input audit-results-xxx.csv
fst report summary --input delete-plan-xxx.csv --top 20
```

The summary includes row counts, known/unknown sizes, delete candidates, confirmed rows, type breakdown, status breakdown, and top largest rows.

## Delete apply

Dry run:

```bash
fst delete apply --input delete-plan-xxx.csv
```

Actually delete:

```bash
fst delete apply --input delete-plan-xxx.csv --yes
```

Safety rules:

- Default is dry-run.
- `--yes` is required for real deletion.
- Only rows with both flags are deleted.

```text
delete_candidate = true
human_confirmed = true
```

Supported delete types:

```text
file
docx
bitable
doc
sheet
mindnote
folder
shortcut
slides
```

Delete results:

```text
delete-results-<timestamp>.json
delete-results-<timestamp>.csv
```

Deleted resources go to Feishu recycle bin. Folder deletion may be asynchronous according to `lark-cli drive +delete` behavior.

## Recommended workflows

### Fast metadata scan

```bash
fst doctor
fst list --limit 1000
fst audit size --mode metadata --input ~/.fst/storage/reports/list-xxx.csv
```

### Accurate native-doc audit

```bash
fst list --doc-types docx,doc,sheet,bitable,slides
fst audit size --mode full --input ~/.fst/storage/reports/list-xxx.csv
```

### Backup before deletion

```bash
fst backup export --input ~/.fst/storage/reports/list-xxx.csv
fst backup download-files --input ~/.fst/storage/reports/list-xxx.csv
```

### Human-confirmed cleanup

```bash
fst audit size --mode full --input ~/.fst/storage/reports/list-xxx.csv
fst report summary --input ~/.fst/storage/reports/audit-results-xxx.csv
fst delete plan --input ~/.fst/storage/reports/top-large-docs-xxx.csv --min-mb 500
```

Then edit `delete-plan-xxx.csv` manually.

```bash
fst delete apply --input ~/.fst/storage/reports/delete-plan-xxx.csv
fst delete apply --input ~/.fst/storage/reports/delete-plan-xxx.csv --yes
```

## Output layout

With default config:

```text
~/.fst/storage/
  backups/
    exports/
    files/
    media/
  reports/
    list-<timestamp>.json
    list-<timestamp>.csv
    manifest-<timestamp>.json
    size-results-metadata-<timestamp>.json
    size-results-metadata-<timestamp>.csv
    top-large-metadata-<timestamp>.csv
    audit-results-<timestamp>.json
    audit-results-<timestamp>.csv
    top-large-docs-<timestamp>.csv
    failed-<timestamp>.json
    file-download-results-<timestamp>.json
    file-download-results-<timestamp>.csv
    delete-plan-<timestamp>.json
    delete-plan-<timestamp>.csv
    delete-results-<timestamp>.json
    delete-results-<timestamp>.csv
```

The workspace root is configurable via `[storage].root`. Point it at another
disk if `~/.fst` is on a small partition:

```toml
[storage]
root = "D:/backup/feishu/fst-storage"
# or
root = "/mnt/nas/feishu/fst-storage"
```

## Authentication and scopes

`fst` calls `lark-cli` directly and uses `--as user` for user-owned Drive resources.

Useful auth commands:

```bash
lark-cli auth login --domain docs
lark-cli auth login --scope "search:docs:read"
lark-cli auth login --scope "docs:document:export"
lark-cli auth login --scope "docx:document:readonly"
lark-cli auth login --scope "docs:document.media:download"
```

Common scopes:

```text
search:docs:read
docs:document:export
docx:document:readonly
docs:document.media:download
drive:drive:readonly
drive:drive
```

If `lark-cli` returns auth/scope errors, `fst` preserves the original output and appends a short next-step hint.

## Limitations

- `doctor` cannot prove every file is accessible. Feishu permissions are per-resource.
- Metadata size is incomplete for Feishu-native docs. Export/full modes are needed for accurate native-doc ranking.
- Embedded resource detection currently focuses on markdown media tags: `source`, `img`, `image`, `file`, `whiteboard`.
- `delete apply` does not delete wiki rows unless they were resolved to supported Drive object types by `delete plan --inspect-wiki`.
- Deletion is intentionally conservative and serial.

`--verbose` enables debug logging. `RUST_LOG` is also supported.

## Exit codes

fst distinguishes failure modes via process exit codes so shell scripts can
react:

| Code | Meaning |
|---:|---|
| 0 | success |
| 1 | uncategorized / internal error (anyhow fallback) |
| 2 | usage error (bad args, missing config, invalid input) |
| 3 | auth failed (need re-login) |
| 4 | missing scope (need scope grant) |
| 5 | network/IO error (transient) |
| 6 | lark-cli invocation failure (non-transient subprocess error) |

clap argument-parsing errors (exit 2) and panics (printed as `fatal: internal
panic at <file>:<line>:<col>` on stderr) bypass this mapping.

Example shell wrapper:

```bash
fst audit size --mode export --input list-xxx.csv
rc=$?
if [ $rc -eq 3 ]; then lark-cli auth login --domain docs
elif [ $rc -eq 4 ]; then lark-cli auth login --scope "$missing"
elif [ $rc -eq 5 ]; then sleep 30; fst audit size --mode export --input list-xxx.csv
fi
```

## Command summary

```bash
fst completions powershell
fst completions bash
fst completions zsh

fst config init
fst config show

fst doctor
fst doctor --doc <doc_url_or_token>
fst doctor --doc-type docx --doc-token <token>

fst list
fst list --folder-token ""
fst list --folder-token <folder_token> --recursive

fst audit size --mode metadata
fst audit size --mode export
fst audit size --mode full
fst audit export

fst backup export
fst backup download-files --input <list.csv>

fst report summary --input <report.csv>

fst delete plan --input <audit-or-list.csv>
fst delete plan --input <audit-or-list.csv> --inspect-wiki
fst delete apply --input <delete-plan.csv>
fst delete apply --input <delete-plan.csv> --yes
```
