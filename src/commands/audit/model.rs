//! Data models shared across audit subcommands.

use serde::{Deserialize, Serialize};

use crate::util::bytes_to_mb;

/// Doc types that `lark-cli drive +export` can handle.
pub const EXPORTABLE_TYPES: &[&str] = &["doc", "docx", "sheet", "bitable", "slides"];

/// File extension lark-cli uses when exporting a given doc type.
pub fn export_extension(doc_type: &str) -> &'static str {
    match doc_type {
        "sheet" => "xlsx",
        "bitable" => "base",
        "slides" => "pptx",
        _ => "docx",
    }
}

/// A doc row as read from a previous fst CSV (list/audit) report.
///
/// Field aliases are resolved at parse time, so downstream code can rely on
/// canonical names.
#[derive(Debug, Clone)]
pub struct RawDoc {
    pub title: String,
    pub doc_type: String,
    pub token: String,
    pub url: String,
    pub update_time_iso: Option<String>,
    pub create_time_iso: Option<String>,
    pub last_open_time_iso: Option<String>,
    pub entity_type: String,
    pub metadata_bytes: Option<u64>,
}

/// Row produced by `audit size --mode metadata`.
#[derive(Debug, Clone, Serialize)]
pub struct MetadataSizeResult {
    pub index: usize,
    pub title: String,
    pub doc_type: String,
    pub entity_type: String,
    pub token: String,
    pub url: String,
    pub update_time_iso: Option<String>,
    pub create_time_iso: Option<String>,
    pub last_open_time_iso: Option<String>,
    pub metadata_bytes: Option<u64>,
    pub metadata_mb: Option<f64>,
    pub size_source: String,
    pub size_confidence: String,
    pub delete_candidate: bool,
    pub human_confirmed: bool,
}

impl MetadataSizeResult {
    pub fn from_raw(index: usize, doc: RawDoc) -> Self {
        let has_size = doc.metadata_bytes.is_some();
        Self {
            index,
            metadata_mb: doc.metadata_bytes.map(bytes_to_mb),
            size_source: if has_size { "metadata" } else { "unknown" }.to_string(),
            size_confidence: if has_size { "metadata_only" } else { "unknown" }.to_string(),
            title: doc.title,
            doc_type: doc.doc_type,
            entity_type: doc.entity_type,
            token: doc.token,
            url: doc.url,
            update_time_iso: doc.update_time_iso,
            create_time_iso: doc.create_time_iso,
            last_open_time_iso: doc.last_open_time_iso,
            metadata_bytes: doc.metadata_bytes,
            delete_candidate: false,
            human_confirmed: false,
        }
    }
}

/// Resolved manifest entry fed to the export worker.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocItem {
    pub index: usize,
    pub title: String,
    pub doc_type: String,
    pub token: String,
    pub url: String,
    pub update_time_iso: Option<String>,
    pub create_time_iso: Option<String>,
    pub last_open_time_iso: Option<String>,
    pub entity_type: String,
    pub export_file: String,
    /// Size hint from upstream metadata (Drive search / previous audit CSV).
    /// When present, `export_doc` uses it to detect truncated/incomplete
    /// local files and re-download them on resume. `None` means unknown;
    /// the existence check alone is used.
    pub expected_bytes: Option<u64>,
}

/// Row produced by `audit size --mode export|full` and `backup export`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditResult {
    pub index: usize,
    pub title: String,
    pub doc_type: String,
    pub token: String,
    pub url: String,
    pub update_time_iso: Option<String>,
    pub create_time_iso: Option<String>,
    pub last_open_time_iso: Option<String>,
    pub export_bytes: Option<u64>,
    pub export_mb: Option<f64>,
    pub media_count: usize,
    pub media_downloaded: usize,
    pub media_bytes: Option<u64>,
    pub media_mb: Option<f64>,
    pub total_bytes: Option<u64>,
    pub total_mb: Option<f64>,
    pub saved_path: Option<String>,
    pub media_dir: Option<String>,
    pub status: String,
    pub error: Option<String>,
    pub delete_candidate: bool,
    pub human_confirmed: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct MediaItem {
    pub doc_index: usize,
    pub doc_title: String,
    pub doc_token: String,
    pub doc_url: String,
    pub token: String,
    pub tag: String,
    pub mime: Option<String>,
    pub name: Option<String>,
    pub href: Option<String>,
    pub output_path: Option<String>,
    pub bytes: Option<u64>,
    pub status: String,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct MediaTag {
    pub token: String,
    pub tag: String,
    pub mime: Option<String>,
    pub name: Option<String>,
    pub href: Option<String>,
}

#[derive(Debug, Default)]
pub struct MediaSummary {
    pub count: usize,
    pub downloaded: usize,
    pub bytes: Option<u64>,
    pub dir: Option<String>,
    pub error: Option<String>,
}
