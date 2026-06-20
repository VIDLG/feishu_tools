use std::path::{Path, PathBuf};
use std::sync::Arc;

use anstream::println;
use anyhow::Result;
use clap::{Args, Subcommand};
use serde::Serialize;

use crate::commands::audit::export::ExportAuditCommand;
use crate::concurrency::{new_progress_bar, run_concurrent};
use crate::config::{AppConfig, FILES_DIR};
use crate::csvutil;
use crate::lark::{LarkCli, append_hint, format_error_with_hint};
use crate::report::{write_csv, write_json};
use crate::util::{file_is_complete, file_size, safe_file_part, timestamp_for_filename};
use fs_err as fs;

#[derive(Debug, Args)]
pub struct BackupCommand {
    #[command(subcommand)]
    command: BackupSubcommand,
}

impl BackupCommand {
    pub async fn run(self, config: &AppConfig) -> Result<()> {
        match self.command {
            BackupSubcommand::Export(command) => command.run(config).await,
            BackupSubcommand::DownloadFiles(command) => command.run(config).await,
        }
    }
}

#[derive(Debug, Subcommand)]
enum BackupSubcommand {
    /// Export Feishu-native docs as local backup files.
    Export(BackupExportCommand),

    /// Download ordinary Drive files listed by fst list.
    DownloadFiles(BackupDownloadFilesCommand),
}

#[derive(Debug, Args)]
struct BackupExportCommand {
    /// Workspace root. Overrides [storage].root in config.toml.
    #[arg(long)]
    root: Option<PathBuf>,

    /// Only process the first N exportable docs. 0 means all search results.
    #[arg(long, default_value_t = 0)]
    limit: usize,

    /// Re-export files and re-download media even when local files exist.
    #[arg(long)]
    force: bool,

    /// Include embedded media/source/file/whiteboard resources. Defaults to true for backup.
    #[arg(long, num_args = 0..=1, default_missing_value = "true")]
    include_media: Option<bool>,

    /// Number of largest rows written to top-large-docs CSV.
    #[arg(long)]
    top_limit: Option<usize>,

    /// Input CSV from fst list or previous audit output. If omitted, search Drive.
    #[arg(long)]
    input: Option<PathBuf>,

    /// Search page size used by lark-cli drive +search.
    #[arg(long)]
    page_size: Option<usize>,
}

impl BackupExportCommand {
    async fn run(self, config: &AppConfig) -> Result<()> {
        ExportAuditCommand {
            root: self.root,
            limit: self.limit,
            force: self.force,
            include_media: Some(self.include_media.unwrap_or(true)),
            top_limit: self.top_limit,
            input: self.input,
            page_size: self.page_size,
        }
        .run(config)
        .await
    }
}

#[derive(Debug, Args)]
struct BackupDownloadFilesCommand {
    /// Input CSV from fst list output.
    #[arg(long)]
    input: PathBuf,

    /// Workspace root. Overrides [storage].root in config.toml.
    #[arg(long)]
    root: Option<PathBuf>,

    /// Only process the first N files. 0 means all file rows.
    #[arg(long, default_value_t = 0)]
    limit: usize,

    /// Re-download files even when local files exist.
    #[arg(long)]
    force: bool,
}

#[derive(Debug, Serialize)]
struct FileDownloadResult {
    title: String,
    token: String,
    url: String,
    output_path: Option<String>,
    bytes: Option<u64>,
    status: String,
    error: Option<String>,
}

impl BackupDownloadFilesCommand {
    async fn run(self, config: &AppConfig) -> Result<()> {
        let workspace_root = self.root.unwrap_or_else(|| config.storage.root.clone());
        let workspace_root_arc = Arc::new(workspace_root);
        let lark = Arc::new(LarkCli::new());
        fs::create_dir_all(workspace_root_arc.join(FILES_DIR))?;
        let reports_dir = workspace_root_arc.join(crate::config::REPORTS_DIR);
        fs::create_dir_all(&reports_dir)?;

        let rows = csvutil::read_csv_maps(&self.input)?;
        let file_rows: Vec<csvutil::CsvRow> = rows
            .into_iter()
            .filter(|row| {
                normalize_type(&csvutil::get(row, &["doc_type", "type", "entity_type"])) == "file"
            })
            .collect();
        let file_rows = if self.limit > 0 && file_rows.len() > self.limit {
            file_rows.into_iter().take(self.limit).collect()
        } else {
            file_rows
        };

        let concurrency = config.audit.concurrency;
        let total = file_rows.len();
        let progress = new_progress_bar(total);
        let multi = indicatif::MultiProgress::new();
        let force = self.force;

        let results = run_concurrent(
            file_rows,
            concurrency,
            Some(progress.clone()),
            Some(multi),
            Some(&|row: &csvutil::CsvRow| csvutil::get(row, &["title"])),
            move |_, row: csvutil::CsvRow| {
                let lark = lark.clone();
                let workspace_root = workspace_root_arc.clone();
                async move { download_one(&lark, &row, &workspace_root, FILES_DIR, force).await }
            },
        )
        .await;
        progress.finish_with_message("done");

        let timestamp = timestamp_for_filename();
        let json_path = reports_dir.join(format!("file-download-results-{timestamp}.json"));
        let csv_path = reports_dir.join(format!("file-download-results-{timestamp}.csv"));
        write_json(&json_path, &results)?;
        write_csv(&csv_path, &results)?;
        println!("Downloaded/skipped: {}", results.len());
        println!("JSON: {}", json_path.display());
        println!("CSV : {}", csv_path.display());
        Ok(())
    }
}

async fn download_one(
    lark: &LarkCli,
    row: &csvutil::CsvRow,
    workspace_root: &Path,
    files_dir_rel: &str,
    force: bool,
) -> FileDownloadResult {
    let title = csvutil::get(row, &["title"]);
    let token = csvutil::get(row, &["token", "file_token"]);
    let url = csvutil::get(row, &["url"]);
    let expected_bytes = csvutil::first_known_size_bytes(row);
    let file_name = backup_file_name(&title, &token);
    let output_path = workspace_root.join(files_dir_rel).join(&file_name);
    let output_rel = format!("{files_dir_rel}/{file_name}");
    let mut status = "downloaded".to_string();
    let mut error = None;
    let mut bytes = None;

    if token.is_empty() {
        status = "skipped".to_string();
        error = Some("missing token".to_string());
    } else if !force && file_is_complete(&output_path, expected_bytes) {
        status = "skipped_existing".to_string();
        bytes = file_size(&output_path);
    } else {
        match lark
            .run_in_with_retry(
                [
                    "drive",
                    "+download",
                    "--as",
                    "user",
                    "--file-token",
                    token.as_str(),
                    "--output",
                    output_rel.as_str(),
                ],
                Some(workspace_root),
            )
            .await
        {
            Ok(result) if result.success => {
                bytes = file_size(&output_path);
            }
            Ok(result) => {
                status = "failed".to_string();
                error = Some(append_hint(result.combined.trim().to_string()));
            }
            Err(err) => {
                status = "failed".to_string();
                error = Some(format_error_with_hint(&err));
            }
        }
    }

    FileDownloadResult {
        title,
        token,
        url,
        output_path: Some(output_path.display().to_string()),
        bytes,
        status,
        error,
    }
}

fn backup_file_name(title: &str, token: &str) -> String {
    let path = std::path::Path::new(title);
    let token_part = safe_file_part(token, 16);
    match (
        path.file_stem().and_then(|value| value.to_str()),
        path.extension().and_then(|value| value.to_str()),
    ) {
        (Some(stem), Some(ext)) if !stem.is_empty() && !ext.is_empty() => format!(
            "{}_{}.{}",
            safe_file_part(stem, 80),
            token_part,
            safe_file_part(ext, 16)
        ),
        _ => format!("{}_{}", safe_file_part(title, 80), token_part),
    }
}

fn normalize_type(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}
