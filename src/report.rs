use std::path::Path;

use anyhow::{Context, Result};
use fs_err as fs;
use serde::Serialize;

pub fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let text = serde_json::to_string_pretty(value)?;
    fs::write(path, text).context("write JSON report")
}

pub fn write_csv<T: Serialize>(path: &Path, rows: &[T]) -> Result<()> {
    let file = fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(path)
        .with_context(|| format!("create CSV report {}", path.display()))?;

    let mut writer = csv::WriterBuilder::new()
        .has_headers(true)
        .from_writer(file);
    for row in rows {
        writer.serialize(row)?;
    }
    writer.flush()?;
    Ok(())
}
