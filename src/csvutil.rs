use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};

const SIZE_BYTE_KEYS: &[&str] = &[
    "total_bytes",
    "known_bytes",
    "metadata_bytes",
    "export_bytes",
    "bytes",
];
const KNOWN_SIZE_BYTE_KEYS: &[&str] = &[
    "known_bytes",
    "total_bytes",
    "metadata_bytes",
    "export_bytes",
    "bytes",
];

const KNOWN_SIZE_MB_KEYS: &[&str] = &["known_mb", "total_mb", "metadata_mb", "export_mb", "mb"];

pub type CsvRow = HashMap<String, String>;

pub fn read_csv_maps(path: &Path) -> Result<Vec<CsvRow>> {
    let mut reader =
        csv::Reader::from_path(path).with_context(|| format!("open CSV {}", path.display()))?;
    let headers = reader.headers()?.clone();
    let mut rows = Vec::new();

    for record in reader.records() {
        let record = record?;
        rows.push(
            headers
                .iter()
                .zip(record.iter())
                .map(|(key, value)| (key.trim().to_string(), value.trim().to_string()))
                .collect(),
        );
    }

    Ok(rows)
}

pub fn get(row: &CsvRow, keys: &[&str]) -> String {
    keys.iter()
        .find_map(|key| row.get(*key).filter(|value| !value.trim().is_empty()))
        .cloned()
        .unwrap_or_default()
}

pub fn get_or(row: &CsvRow, keys: &[&str], fallback: &str) -> String {
    let value = get(row, keys);
    if value.is_empty() {
        fallback.to_string()
    } else {
        value
    }
}

pub fn get_optional(row: &CsvRow, keys: &[&str]) -> Option<String> {
    let value = get(row, keys);
    if value.is_empty() { None } else { Some(value) }
}

pub fn first_u64(row: &CsvRow, keys: &[&str]) -> Option<u64> {
    keys.iter()
        .filter_map(|key| row.get(*key))
        .find_map(|value| value.trim().parse::<u64>().ok())
}

pub fn first_f64(row: &CsvRow, keys: &[&str]) -> Option<f64> {
    keys.iter()
        .filter_map(|key| row.get(*key))
        .find_map(|value| value.trim().parse::<f64>().ok())
}

pub fn first_bool(row: &CsvRow, keys: &[&str]) -> Option<bool> {
    keys.iter()
        .filter_map(|key| row.get(*key))
        .find_map(|value| parse_bool(value))
}

pub fn first_size_bytes(row: &CsvRow) -> Option<u64> {
    first_u64(row, SIZE_BYTE_KEYS)
}

pub fn first_known_size_bytes(row: &CsvRow) -> Option<u64> {
    first_u64(row, KNOWN_SIZE_BYTE_KEYS)
}

pub fn first_known_size_mb(row: &CsvRow) -> Option<f64> {
    first_f64(row, KNOWN_SIZE_MB_KEYS)
}

pub fn parse_bool(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" | "t" | "yes" | "y" | "1" | "是" | "确认" | "确认删除" => Some(true),
        "false" | "f" | "no" | "n" | "0" | "否" | "不" | "不删除" | "" => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::parse_bool;

    #[test]
    fn parses_delete_confirmation_bools() {
        assert_eq!(parse_bool("确认删除"), Some(true));
        assert_eq!(parse_bool(" no "), Some(false));
        assert_eq!(parse_bool("maybe"), None);
    }
}
