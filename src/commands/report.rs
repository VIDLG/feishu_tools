use std::collections::BTreeMap;
use std::path::PathBuf;

use anstream::println;
use anyhow::Result;
use clap::{Args, Subcommand};
use tabled::{Table, Tabled};

use crate::csvutil;
use crate::util::format_bytes;

#[derive(Debug, Args)]
pub struct ReportCommand {
    #[command(subcommand)]
    command: ReportSubcommand,
}

impl ReportCommand {
    pub fn run(self) -> Result<()> {
        match self.command {
            ReportSubcommand::Summary(command) => command.run(),
        }
    }
}

#[derive(Debug, Subcommand)]
enum ReportSubcommand {
    /// Summarize an fst CSV report by type, known size, status, and candidates.
    Summary(ReportSummaryCommand),
}

#[derive(Debug, Args)]
struct ReportSummaryCommand {
    /// Input CSV report.
    #[arg(long)]
    input: PathBuf,

    /// Number of largest rows to show.
    #[arg(long, default_value_t = 10)]
    top: usize,
}

#[derive(Debug, Tabled)]
struct TypeRow {
    doc_type: String,
    rows: usize,
    known_size: String,
}

#[derive(Debug, Tabled)]
struct StatusRow {
    status: String,
    rows: usize,
}

#[derive(Debug, Tabled)]
struct TopRow {
    rank: usize,
    title: String,
    doc_type: String,
    size: String,
    url: String,
}

impl ReportSummaryCommand {
    fn run(self) -> Result<()> {
        let rows = csvutil::read_csv_maps(&self.input)?;
        let total_rows = rows.len();
        let mut known_rows = 0usize;
        let mut total_bytes = 0u64;
        let mut candidate_rows = 0usize;
        let mut confirmed_rows = 0usize;
        let mut by_type: BTreeMap<String, (usize, u64)> = BTreeMap::new();
        let mut by_status: BTreeMap<String, usize> = BTreeMap::new();
        let mut top = Vec::new();

        for row in &rows {
            let doc_type = csvutil::get(row, &["doc_type", "type", "entity_type"]);
            let doc_type = if doc_type.is_empty() {
                "unknown".to_string()
            } else {
                doc_type
            };
            let status = csvutil::get(row, &["status"]);
            if !status.is_empty() {
                *by_status.entry(status).or_default() += 1;
            }
            if csvutil::first_bool(row, &["delete_candidate"]).unwrap_or(false) {
                candidate_rows += 1;
            }
            if csvutil::first_bool(row, &["human_confirmed"]).unwrap_or(false) {
                confirmed_rows += 1;
            }
            let bytes = csvutil::first_size_bytes(row);
            if let Some(bytes) = bytes {
                known_rows += 1;
                total_bytes += bytes;
                let entry = by_type.entry(doc_type.clone()).or_default();
                entry.0 += 1;
                entry.1 += bytes;
                top.push((bytes, row, doc_type));
            } else {
                let entry = by_type.entry(doc_type).or_default();
                entry.0 += 1;
            }
        }

        println!("Rows            : {total_rows}");
        println!("Known size rows : {known_rows}");
        println!(
            "Unknown rows    : {}",
            total_rows.saturating_sub(known_rows)
        );
        println!("Known total     : {}", format_bytes(total_bytes));
        println!("Candidates      : {candidate_rows}");
        println!("Confirmed       : {confirmed_rows}");

        let type_rows = by_type
            .into_iter()
            .map(|(doc_type, (rows, bytes))| TypeRow {
                doc_type,
                rows,
                known_size: format_bytes(bytes),
            })
            .collect::<Vec<_>>();
        if !type_rows.is_empty() {
            println!();
            println!("By type");
            println!("{}", Table::new(type_rows));
        }

        let status_rows = by_status
            .into_iter()
            .map(|(status, rows)| StatusRow { status, rows })
            .collect::<Vec<_>>();
        if !status_rows.is_empty() {
            println!();
            println!("By status");
            println!("{}", Table::new(status_rows));
        }

        top.sort_by_key(|item| std::cmp::Reverse(item.0));
        let top_rows = top
            .into_iter()
            .take(self.top)
            .enumerate()
            .map(|(idx, (bytes, row, doc_type))| TopRow {
                rank: idx + 1,
                title: csvutil::get(row, &["title"]),
                doc_type,
                size: format_bytes(bytes),
                url: csvutil::get(row, &["url"]),
            })
            .collect::<Vec<_>>();
        if !top_rows.is_empty() {
            println!();
            println!("Top {}", top_rows.len());
            println!("{}", Table::new(top_rows));
        }

        Ok(())
    }
}
