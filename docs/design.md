# Design notes

Short notes for future maintainers. These are decisions, not promises.

## API backend: use `lark-cli`, not a Rust SDK

Feishu/Lark Rust SDK coverage is uneven, while `lark-cli` already handles auth,
scopes, exports, downloads, and destructive confirmation flags. `fst` stays a
small Rust orchestrator around that:

- CLI shape and config
- report files
- sorting and summaries
- backup paths
- delete-plan safety

Add direct API calls only when `lark-cli` cannot do the job.

## External Lark JSON: use `serde_json::Value`

For Lark responses, `Value` is intentional. Different `lark-cli` commands expose
slightly different shapes and aliases, such as `token` vs `file_token`, `name`
vs `title`, or `page_token` vs `next_page_token`.

Strong structs would mostly be optional fields and aliases. That is more code
for less flexibility. Keep this boundary dynamic and hide the noise behind tiny
helpers like `json_string()`, `json_string_any()`, and `metadata_bytes()`.

Use typed `serde` structs for data we own: config, reports, delete plans, and
results.

## CSV: use the `csv` crate, not manual parsing

CSV has quoting, escaping, commas inside values, and spreadsheet edits. The
standard library does not parse CSV. The `csv` crate is small, common, and
already solves the edge cases.

Rows are read as `HashMap<String, String>` because reports may come from
multiple commands or older runs with slightly different headers. Helpers in
`csvutil` define the accepted aliases and keep field priority explicit.

## Size fields: preserve priority

Do not blindly merge all size lookup lists. Field order can affect behavior.
For example, delete-plan input prefers `known_bytes` first, while summary output
prefers `total_bytes` first.

If a shared helper changes priority, do not use it.

## Delete safety

Deletion requires both:

- `delete_candidate = true`
- `human_confirmed = true`

`--yes` is still required to execute. Keep this boring. Data loss is not where
we get clever.

## Tests

Keep tests tiny and targeted. Add one test when a parser or safety rule would be
easy to break silently. Avoid fixtures unless a real bug needs them.
