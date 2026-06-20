use fs_err as fs;
use humansize::{BINARY, format_size};
use jiff::Zoned;
use regex::Regex;
use serde_json::Value;
use std::path::Path;
use std::sync::LazyLock;

static WHITESPACE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\s+").expect("valid whitespace regex"));
static HTML_TAG_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<[^>]+>").expect("valid tag-strip regex"));

pub fn safe_file_part(input: &str, max_chars: usize) -> String {
    let fallback = "Untitled document";
    let normalized = WHITESPACE_RE.replace_all(input.trim(), " ");
    let sanitized = sanitize_filename::sanitize_with_options(
        normalized.as_ref(),
        sanitize_filename::Options {
            truncate: false,
            windows: true,
            replacement: "_",
        },
    );
    let value = sanitized.trim().trim_matches('.');
    let value = if value.is_empty() { fallback } else { value };
    value
        .chars()
        .take(max_chars)
        .collect::<String>()
        .trim()
        .to_string()
}

pub fn extension_from_mime(mime: Option<&str>, fallback_tag: &str) -> &'static str {
    match mime
        .unwrap_or_default()
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
    {
        "image/png" => "png",
        "image/jpeg" | "image/jpg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "video/mp4" => "mp4",
        "video/quicktime" => "mov",
        "application/pdf" => "pdf",
        "application/vnd.openxmlformats-officedocument.presentationml.presentation" => "pptx",
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document" => "docx",
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet" => "xlsx",
        "application/zip" => "zip",
        _ if fallback_tag == "whiteboard" => "png",
        _ => "bin",
    }
}

pub fn file_size(path: &Path) -> Option<u64> {
    fs::metadata(path).ok().map(|metadata| metadata.len())
}

/// Decide whether a previously-downloaded file can be safely skipped.
///
/// - If `expected_bytes` is unknown, only existence is checked (the original
///   behavior). This avoids re-downloading when no size hint is available.
/// - If `expected_bytes` is known, the file must both exist AND match the
///   expected size; otherwise it is treated as truncated/corrupt and will be
///   re-downloaded.
pub fn file_is_complete(path: &Path, expected_bytes: Option<u64>) -> bool {
    let Some(actual) = file_size(path) else {
        return false;
    };
    match expected_bytes {
        Some(expected) => actual == expected,
        None => true,
    }
}

pub fn bytes_to_mb(bytes: u64) -> f64 {
    (bytes as f64 / 1024.0 / 1024.0 * 100.0).round() / 100.0
}

pub fn format_bytes(bytes: u64) -> String {
    format_size(bytes, BINARY)
}

pub fn timestamp_for_filename() -> String {
    Zoned::now().strftime("%Y%m%d-%H%M%S").to_string()
}

pub fn strip_html_tags(input: &str) -> String {
    HTML_TAG_RE.replace_all(input, "").to_string()
}

pub fn highlighted_title(value: &Value) -> String {
    let title_html = value
        .get("title_highlighted")
        .and_then(Value::as_str)
        .unwrap_or_default();
    html_escape::decode_html_entities(&strip_html_tags(title_html)).to_string()
}

pub fn metadata_bytes(value: &Value) -> Option<u64> {
    value
        .get("size")
        .or_else(|| value.get("size_bytes"))
        .or_else(|| value.get("file_size"))
        .or_else(|| value.get("bytes"))
        .and_then(|value| value.as_u64().or_else(|| value.as_str()?.parse().ok()))
}

pub fn json_string(value: &Value, key: &str) -> String {
    json_optional_string(value, key).unwrap_or_default()
}

pub fn json_string_any(value: &Value, keys: &[&str]) -> String {
    keys.iter()
        .find_map(|key| json_optional_string(value, key))
        .unwrap_or_default()
}

pub fn json_optional_string(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

pub fn json_pointer_string(value: &Value, pointer: &str) -> String {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

pub fn json_optional_pointer_string(value: &Value, pointer: &str) -> Option<String> {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

#[cfg(test)]
mod tests {
    use super::file_is_complete;
    use std::path::Path;

    #[test]
    fn file_is_complete_returns_false_for_missing_file() {
        assert!(!file_is_complete(Path::new("/no/such/path/at-all"), None));
        assert!(!file_is_complete(
            Path::new("/no/such/path/at-all"),
            Some(100),
        ));
    }

    #[test]
    fn file_is_complete_with_unknown_size_accepts_any_present_file() {
        // Project's own Cargo.toml is guaranteed to exist and be non-empty.
        let existing = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
        assert!(file_is_complete(&existing, None));
    }

    #[test]
    fn file_is_complete_with_known_size_requires_exact_match() {
        let existing = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
        let actual = super::fs::metadata(&existing).unwrap().len();
        assert!(file_is_complete(&existing, Some(actual)));
        assert!(!file_is_complete(&existing, Some(actual + 1)));
        assert!(!file_is_complete(&existing, Some(0)));
    }
}
