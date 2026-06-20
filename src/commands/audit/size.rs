//! `audit size --mode metadata` implementation.

use std::cmp::Reverse;
use std::path::PathBuf;

use anstream::println;
use anyhow::Result;
use clap::Args;

use crate::config::AppConfig;
use crate::lark::{LarkCli, check_auth};
use crate::report::{write_csv, write_json};
use crate::util::{format_bytes, timestamp_for_filename};
use fs_err as fs;

use super::common::{read_raw_docs_from_csv, search_docs};
use super::model::MetadataSizeResult;

/// Common scopes required by audit commands.
pub const AUDIT_REQUIRED_SCOPES: &[&str] = &[
    "docx:document:readonly",
    "docs:document:export",
    "docs:document.media:download",
    "search:docs:read",
];

#[derive(Debug, Clone)]
pub struct SizeAuditCommand {
    pub mode: super::SizeAuditMode,
    pub root: Option<PathBuf>,
    pub limit: usize,
    pub input: Option<PathBuf>,
    pub page_size: Option<usize>,
    pub top_limit: Option<usize>,
    pub force: bool,
}

impl SizeAuditCommand {
    pub async fn run(self, config: &AppConfig) -> Result<()> {
        match self.mode {
            super::SizeAuditMode::Metadata => run_metadata_size_audit(self, config).await,
            super::SizeAuditMode::Export | super::SizeAuditMode::Full => {
                let include_media = matches!(self.mode, super::SizeAuditMode::Full);
                super::export::ExportAuditCommand {
                    root: self.root,
                    limit: self.limit,
                    force: self.force,
                    include_media: Some(include_media),
                    top_limit: self.top_limit,
                    input: self.input,
                    page_size: self.page_size,
                }
                .run(config)
                .await
            }
        }
    }
}

/// CLI mirror of `audit size` (kept thin so the `audit` mod root stays small).
#[derive(Debug, Args)]
pub struct SizeAuditCli {
    /// Audit mode.
    #[arg(long, value_enum, default_value_t = super::SizeAuditMode::Metadata)]
    pub mode: super::SizeAuditMode,

    /// Workspace root. Overrides [storage].root in config.toml.
    #[arg(long)]
    pub root: Option<PathBuf>,

    /// Only process the first N docs. 0 means all search results.
    #[arg(long, default_value_t = 0)]
    pub limit: usize,

    /// Input CSV from fst list or previous audit output. If omitted, search Drive.
    #[arg(long)]
    pub input: Option<PathBuf>,

    /// Search page size used by lark-cli drive +search.
    #[arg(long)]
    pub page_size: Option<usize>,

    /// Number of largest rows written to top-large-metadata CSV.
    #[arg(long)]
    pub top_limit: Option<usize>,

    /// Re-export files even when local files exist. Ignored by metadata mode.
    #[arg(long)]
    pub force: bool,
}

impl SizeAuditCli {
    pub fn into_command(self) -> SizeAuditCommand {
        SizeAuditCommand {
            mode: self.mode,
            root: self.root,
            limit: self.limit,
            input: self.input,
            page_size: self.page_size,
            top_limit: self.top_limit,
            force: self.force,
        }
    }
}

async fn run_metadata_size_audit(command: SizeAuditCommand, config: &AppConfig) -> Result<()> {
    let workspace_root = command.root.unwrap_or_else(|| config.storage.root.clone());
    let reports_dir = workspace_root.join(crate::config::REPORTS_DIR);
    fs::create_dir_all(&reports_dir)?;

    let timestamp = timestamp_for_filename();
    let results_json_path = reports_dir.join(format!("size-results-metadata-{timestamp}.json"));
    let results_csv_path = reports_dir.join(format!("size-results-metadata-{timestamp}.csv"));
    let top_csv_path = reports_dir.join(format!("top-large-metadata-{timestamp}.csv"));

    let lark = LarkCli::new();
    check_auth(&lark, AUDIT_REQUIRED_SCOPES).await?;
    let page_size = command.page_size.unwrap_or(config.list.page_size);
    let top_limit = command.top_limit.unwrap_or(config.audit.top_limit);
    let docs = if let Some(input) = &command.input {
        let mut docs = read_raw_docs_from_csv(input, false)?;
        if command.limit > 0 && docs.len() > command.limit {
            docs.truncate(command.limit);
        }
        docs
    } else {
        search_docs(&lark, command.limit, page_size, false).await?
    };

    let mut rows: Vec<_> = docs
        .into_iter()
        .enumerate()
        .map(|(offset, doc)| MetadataSizeResult::from_raw(offset + 1, doc))
        .collect();

    rows.sort_by_key(|row| Reverse(row.metadata_bytes));
    let known = rows
        .iter()
        .filter(|row| row.metadata_bytes.is_some())
        .count();
    let total_bytes: u64 = rows.iter().map(|row| row.metadata_bytes.unwrap_or(0)).sum();
    write_json(&results_json_path, &rows)?;
    write_csv(&results_csv_path, &rows)?;
    let top_rows: Vec<_> = rows.iter().take(top_limit).cloned().collect();
    write_csv(&top_csv_path, &top_rows)?;

    println!("Done.");
    println!("Resources        : {}", rows.len());
    println!("Known sizes      : {}", known);
    println!("Unknown sizes    : {}", rows.len().saturating_sub(known));
    println!("Known total size : {}", format_bytes(total_bytes));
    println!("Results JSON     : {}", results_json_path.display());
    println!("Results CSV      : {}", results_csv_path.display());
    println!("Top CSV          : {}", top_csv_path.display());
    println!();
    println!("Largest metadata-known resources:");
    for row in rows
        .iter()
        .filter(|row| row.metadata_bytes.is_some())
        .take(10)
    {
        println!(
            "  #{:03} {:>10.2} MB  {}  {}",
            row.index,
            row.metadata_mb.unwrap_or(0.0),
            row.title,
            row.url
        );
    }

    Ok(())
}
