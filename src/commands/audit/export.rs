//! `audit size --mode export|full`, `audit export`, and `backup export`.
//!
//! One module instead of `export.rs` + `export_command.rs` because the worker
//! (`export_doc`) and the orchestrator (`ExportAuditCommand::run`) are tightly
//! coupled by the [`AuditResult`] shape and the media contract; splitting them
//! only made call sites harder to follow.

use std::cmp::Reverse;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anstream::println;
use anyhow::Result;
use clap::Args;
use fs_err as fs;
use serde_json::Value;

use crate::concurrency::{new_progress_bar, run_concurrent};
use crate::config::{AppConfig, EXPORTS_DIR, MEDIA_DIR};
use crate::lark::{LarkCli, append_hint, check_auth, format_error_with_hint, parse_json_from_text};
use crate::report::{write_csv, write_json};
use crate::util::{
    bytes_to_mb, file_is_complete, file_size, format_bytes, json_optional_pointer_string,
    timestamp_for_filename,
};

use super::common::{build_manifest, read_raw_docs_from_csv, search_docs};
use super::media::download_doc_media;
use super::model::{AuditResult, DocItem, MediaSummary, export_extension};
use super::size::AUDIT_REQUIRED_SCOPES;

// ---------------------------------------------------------------------
// Worker: export a single doc (and optionally its embedded media).
// ---------------------------------------------------------------------

/// Export one docx/sheet/bitable/slides doc via `lark-cli drive +export`,
/// optionally fetching embedded media.
///
/// `workspace_root` is the directory lark-cli will be spawned in. All
/// `--output-dir` / `--output` args are passed as paths relative to it,
/// because lark-cli refuses absolute output paths (path-traversal guard).
/// Local filesystem checks use absolute paths joined from `workspace_root`.
pub async fn export_doc(
    lark: &LarkCli,
    doc: &DocItem,
    workspace_root: &Path,
    exports_dir_rel: &str,
    media_dir_rel: Option<&str>,
    force: bool,
) -> AuditResult {
    let exports_dir_abs = workspace_root.join(exports_dir_rel);
    let full_path = exports_dir_abs.join(&doc.export_file);
    let mut status = "exported".to_string();
    let mut error: Option<String> = None;
    let mut export_bytes = None;
    let mut saved_path = None;

    if !force && file_is_complete(&full_path, doc.expected_bytes) {
        status = "skipped_existing".to_string();
        export_bytes = file_size(&full_path);
        saved_path = Some(full_path.display().to_string());
    } else {
        let ext = export_extension(&doc.doc_type);
        let output = lark
            .run_in_with_retry(
                [
                    "drive",
                    "+export",
                    "--as",
                    "user",
                    "--doc-type",
                    doc.doc_type.as_str(),
                    "--token",
                    doc.token.as_str(),
                    "--file-extension",
                    ext,
                    "--file-name",
                    doc.export_file.as_str(),
                    "--output-dir",
                    exports_dir_rel,
                    "--overwrite",
                ],
                Some(workspace_root),
            )
            .await;

        match output {
            Ok(output) if output.success => {
                match parse_json_from_text(&output.combined) {
                    Ok(parsed) => {
                        export_bytes = parsed.pointer("/data/size_bytes").and_then(Value::as_u64);
                        saved_path = json_optional_pointer_string(&parsed, "/data/saved_path");
                    }
                    Err(parse_error) => {
                        error = Some(format!(
                            "export succeeded but JSON parsing failed: {parse_error}"
                        ))
                    }
                }
                if export_bytes.is_none() {
                    export_bytes = file_size(&full_path);
                }
                if saved_path.is_none() && full_path.exists() {
                    saved_path = Some(full_path.display().to_string());
                }
            }
            Ok(output) => {
                status = "failed".to_string();
                error = Some(append_hint(output.combined.trim().to_string()));
            }
            Err(err) => {
                status = "failed".to_string();
                error = Some(format_error_with_hint(&err));
            }
        }
    }

    let media_summary = if let Some(media_dir_rel) = media_dir_rel {
        download_doc_media(lark, doc, workspace_root, media_dir_rel, force, None).await
    } else {
        MediaSummary::default()
    };

    if let Some(media_error) = media_summary.error.as_ref() {
        error = Some(match error {
            Some(existing) => format!("{existing}\n{media_error}"),
            None => media_error.clone(),
        });
    }

    let total_bytes = match (export_bytes, media_summary.bytes) {
        (Some(export), Some(media)) => Some(export + media),
        (Some(export), None) => Some(export),
        (None, Some(media)) => Some(media),
        (None, None) => None,
    };

    AuditResult {
        index: doc.index,
        title: doc.title.clone(),
        doc_type: doc.doc_type.clone(),
        token: doc.token.clone(),
        url: doc.url.clone(),
        update_time_iso: doc.update_time_iso.clone(),
        create_time_iso: doc.create_time_iso.clone(),
        last_open_time_iso: doc.last_open_time_iso.clone(),
        export_bytes,
        export_mb: export_bytes.map(bytes_to_mb),
        media_count: media_summary.count,
        media_downloaded: media_summary.downloaded,
        media_bytes: media_summary.bytes,
        media_mb: media_summary.bytes.map(bytes_to_mb),
        total_bytes,
        total_mb: total_bytes.map(bytes_to_mb),
        saved_path,
        media_dir: media_summary.dir,
        status,
        error,
        delete_candidate: false,
        human_confirmed: false,
    }
}

// ---------------------------------------------------------------------
// Orchestrator: drives the export loop over many docs.
// ---------------------------------------------------------------------

/// CLI mirror of `audit export`. Field-for-field identical to the args of
/// [`ExportAuditCommand`]; the From impl below is the bridge.
#[derive(Debug, Args)]
pub struct ExportAuditCli {
    /// Workspace root. Overrides [storage].root in config.toml.
    #[arg(long)]
    pub root: Option<PathBuf>,

    /// Only process the first N exportable docs. 0 means all search results.
    #[arg(long, default_value_t = 0)]
    pub limit: usize,

    /// Re-export files and re-download media even when local files exist.
    #[arg(long)]
    pub force: bool,

    /// Also fetch document markdown and download embedded media/source/file/whiteboard resources.
    #[arg(long, num_args = 0..=1, default_missing_value = "true")]
    pub include_media: Option<bool>,

    /// Number of largest rows written to top-large-docs CSV.
    #[arg(long)]
    pub top_limit: Option<usize>,

    /// Input CSV from fst list or previous audit output. If omitted, search Drive.
    #[arg(long)]
    pub input: Option<PathBuf>,

    /// Search page size used by lark-cli drive +search.
    #[arg(long)]
    pub page_size: Option<usize>,
}

/// Runtime params shared by `audit size --mode export|full`, `audit export`,
/// and `backup export`.
#[derive(Debug, Clone)]
pub struct ExportAuditCommand {
    pub root: Option<PathBuf>,
    pub limit: usize,
    pub force: bool,
    pub include_media: Option<bool>,
    pub top_limit: Option<usize>,
    pub input: Option<PathBuf>,
    pub page_size: Option<usize>,
}

impl From<ExportAuditCli> for ExportAuditCommand {
    fn from(cli: ExportAuditCli) -> Self {
        Self {
            root: cli.root,
            limit: cli.limit,
            force: cli.force,
            include_media: cli.include_media,
            top_limit: cli.top_limit,
            input: cli.input,
            page_size: cli.page_size,
        }
    }
}

impl ExportAuditCommand {
    pub async fn run(self, config: &AppConfig) -> Result<()> {
        run_export_audit(self, config).await
    }
}

async fn run_export_audit(command: ExportAuditCommand, config: &AppConfig) -> Result<()> {
    let workspace_root = command.root.unwrap_or_else(|| config.storage.root.clone());
    let include_media = command.include_media.unwrap_or(config.audit.include_media);
    let top_limit = command.top_limit.unwrap_or(config.audit.top_limit);
    let page_size = command.page_size.unwrap_or(config.list.page_size);

    let exports_dir = workspace_root.join(EXPORTS_DIR);
    let media_dir = workspace_root.join(MEDIA_DIR);
    let reports_dir = workspace_root.join(crate::config::REPORTS_DIR);
    let timestamp = timestamp_for_filename();

    fs::create_dir_all(&exports_dir)?;
    if include_media {
        fs::create_dir_all(&media_dir)?;
    }
    fs::create_dir_all(&reports_dir)?;

    let lark = LarkCli::new();
    check_auth(&lark, AUDIT_REQUIRED_SCOPES).await?;

    tracing::info!("Workspace root: {}", workspace_root.display());
    tracing::info!("Exports    : {}", exports_dir.display());
    if include_media {
        tracing::info!("Media      : {}", media_dir.display());
    }
    tracing::info!("Reports    : {}", reports_dir.display());

    let docs = if let Some(input) = &command.input {
        let mut docs = read_raw_docs_from_csv(input, true)?;
        if command.limit > 0 && docs.len() > command.limit {
            docs.truncate(command.limit);
        }
        docs
    } else {
        search_docs(&lark, command.limit, page_size, true).await?
    };
    let manifest = build_manifest(docs);
    let manifest_path = reports_dir.join(format!("manifest-{timestamp}.json"));
    write_json(&manifest_path, &manifest)?;
    tracing::info!("Manifest written: {}", manifest_path.display());

    let results_json_path = reports_dir.join(format!("audit-results-{timestamp}.json"));
    let results_csv_path = reports_dir.join(format!("audit-results-{timestamp}.csv"));
    let top_csv_path = reports_dir.join(format!("top-large-docs-{timestamp}.csv"));
    let failures_json_path = reports_dir.join(format!("failed-{timestamp}.json"));

    let progress = new_progress_bar(manifest.len());
    let multi = indicatif::MultiProgress::new();
    let media_dir_rel: Option<&'static str> = if include_media { Some(MEDIA_DIR) } else { None };
    let workspace_root_arc = Arc::new(workspace_root);
    let lark = Arc::new(lark);
    let force = command.force;

    let results = run_concurrent(
        manifest,
        config.audit.concurrency,
        Some(progress.clone()),
        Some(multi),
        Some(&|doc: &DocItem| format!("#{} {}", doc.index, doc.title)),
        move |_, doc: DocItem| {
            let lark = lark.clone();
            let workspace_root = workspace_root_arc.clone();
            async move {
                export_doc(
                    &lark,
                    &doc,
                    &workspace_root,
                    EXPORTS_DIR,
                    media_dir_rel,
                    force,
                )
                .await
            }
        },
    )
    .await;
    progress.finish_with_message("done");

    let mut results = results;
    results.sort_by_key(|row| Reverse(row.total_bytes));
    write_json(&results_json_path, &results)?;
    write_csv(&results_csv_path, &results)?;

    let failures: Vec<_> = results
        .iter()
        .filter(|row| row.status == "failed")
        .collect();
    write_json(&failures_json_path, &failures)?;

    let top_rows: Vec<_> = results.iter().take(top_limit).cloned().collect();
    write_csv(&top_csv_path, &top_rows)?;

    let total_bytes: u64 = results.iter().map(|row| row.total_bytes.unwrap_or(0)).sum();
    println!();
    println!("Done.");
    println!("Processed        : {}", results.len());
    println!("Failed           : {}", failures.len());
    println!("Total known size : {}", format_bytes(total_bytes));
    println!("Results JSON     : {}", results_json_path.display());
    println!("Results CSV      : {}", results_csv_path.display());
    println!("Top CSV          : {}", top_csv_path.display());
    println!("Failures JSON    : {}", failures_json_path.display());
    println!();
    println!("Largest known docs:");
    for row in results.iter().take(10) {
        println!(
            "  #{:03} {:>10.2} MB  {}  {}",
            row.index,
            row.total_mb.unwrap_or(0.0),
            row.title,
            row.url
        );
    }

    Ok(())
}
