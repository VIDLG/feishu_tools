//! `fst audit` command: size audit (metadata/export/full) and audit export.
//!
//! The implementation is split into focused submodules:
//! - [`model`]: shared data types and constants.
//! - [`common`]: CSV ingestion, Drive search, manifest building.
//! - [`media`]: embedded media discovery and download.
//! - [`export`]: per-doc export worker AND export/full-mode orchestrator.
//! - [`size`]: `audit size --mode metadata` and the size subcommand entrypoint.

pub mod common;
pub mod export;
pub mod media;
pub mod model;
pub mod size;

use anyhow::Result;
use clap::{Args, Subcommand, ValueEnum};

use crate::config::AppConfig;

use export::{ExportAuditCli, ExportAuditCommand};
use size::SizeAuditCli;

#[derive(Debug, Args)]
pub struct AuditCommand {
    #[command(subcommand)]
    command: AuditSubcommand,
}

impl AuditCommand {
    pub async fn run(self, config: &AppConfig) -> Result<()> {
        match self.command {
            AuditSubcommand::Size(cli) => cli.into_command().run(config).await,
            AuditSubcommand::Export(cli) => ExportAuditCommand::from(cli).run(config).await,
        }
    }
}

#[derive(Debug, Subcommand)]
enum AuditSubcommand {
    /// Compute per-doc size using metadata, export, or full download modes.
    Size(SizeAuditCli),

    /// Export Feishu-native docs and rank by exported size. Equivalent to
    /// `audit size --mode full` when `--include-media` is enabled, and to
    /// `audit size --mode export` otherwise.
    Export(ExportAuditCli),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum SizeAuditMode {
    /// Fast scan using only Drive metadata size. Reliable mainly for ordinary
    /// Drive files when metadata carries a size.
    Metadata,

    /// Export native docs and rank by exported file size.
    Export,

    /// Export native docs and also download embedded media. Closest to actual
    /// backup size.
    Full,
}
