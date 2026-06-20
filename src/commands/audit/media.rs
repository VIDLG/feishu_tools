//! Embedded media discovery and download for `audit size --mode full`.

use std::collections::HashSet;
use std::path::Path;
use std::sync::LazyLock;

use anyhow::Result;
use html_escape::decode_html_entities;
use indicatif::ProgressBar;
use regex::Regex;

use crate::lark::{LarkCli, append_hint, format_error_with_hint};
use crate::report::{write_csv, write_json};
use crate::util::{
    extension_from_mime, file_is_complete, file_size, json_pointer_string, safe_file_part,
};
use fs_err as fs;

use super::model::{DocItem, MediaItem, MediaSummary, MediaTag};

static MEDIA_TAG_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?is)<(source|img|image|file|whiteboard)\b([^>]*)/?\s*>"#)
        .expect("valid media tag regex")
});
static MEDIA_ATTR_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"([A-Za-z_:][-A-Za-z0-9_:]*)\s*=\s*['\"]([^'\"]*)['\"]"#)
        .expect("valid attr regex")
});

/// Fetch doc markdown and download all embedded media into a per-doc
/// directory under `media_dir_rel` (relative to `workspace_root`).
///
/// `media_dir_rel` is passed verbatim to lark-cli as the `--output` prefix;
/// lark-cli is spawned with `cwd = workspace_root` because it refuses
/// absolute output paths (path-traversal guard).
pub async fn download_doc_media(
    lark: &LarkCli,
    doc: &DocItem,
    workspace_root: &Path,
    media_dir_rel: &str,
    force: bool,
    progress: Option<&ProgressBar>,
) -> MediaSummary {
    let content = match fetch_doc_content(lark, doc).await {
        Ok(content) => content,
        Err(err) => {
            return MediaSummary {
                error: Some(format!("media fetch failed: {err}")),
                ..MediaSummary::default()
            };
        }
    };
    let items = find_media_items(&content);
    if items.is_empty() {
        return MediaSummary {
            bytes: Some(0),
            ..MediaSummary::default()
        };
    }

    let doc_dir_name = format!("audit_{:03}_{}", doc.index, safe_file_part(&doc.title, 60));
    let doc_dir_rel = format!("{media_dir_rel}/{doc_dir_name}");
    let doc_dir_abs = workspace_root.join(&doc_dir_rel);
    if let Err(err) = fs::create_dir_all(&doc_dir_abs) {
        return MediaSummary {
            count: items.len(),
            error: Some(format!("create media dir failed: {err}")),
            ..MediaSummary::default()
        };
    }

    let mut rows = Vec::with_capacity(items.len());
    for (offset, item) in items.iter().enumerate() {
        let index = offset + 1;
        let ext = item
            .name
            .as_ref()
            .and_then(|name| {
                Path::new(name)
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .map(ToOwned::to_owned)
            })
            .unwrap_or_else(|| extension_from_mime(item.mime.as_deref(), &item.tag).to_string());
        let stem = item
            .name
            .as_ref()
            .and_then(|name| Path::new(name).file_stem().and_then(|stem| stem.to_str()))
            .map(|stem| safe_file_part(stem, 50))
            .unwrap_or_else(|| safe_file_part(&format!("{}_{index:03}", item.tag), 50));
        let token_prefix: String = item.token.chars().take(8).collect();
        let output_name = format!("{index:03}_{stem}_{token_prefix}.{ext}");
        let output_path = doc_dir_abs.join(&output_name);
        let output_rel = format!("{doc_dir_rel}/{output_name}");
        let mut status = "downloaded".to_string();
        let mut error = None;
        let mut bytes = None;

        if !force && file_is_complete(&output_path, None) {
            status = "skipped_existing".to_string();
            bytes = file_size(&output_path);
        } else {
            let media_type = if item.tag == "whiteboard" {
                "whiteboard"
            } else {
                "media"
            };
            match lark
                .run_in_with_retry(
                    [
                        "docs",
                        "+media-download",
                        "--as",
                        "user",
                        "--token",
                        item.token.as_str(),
                        "--type",
                        media_type,
                        "--output",
                        output_rel.as_str(),
                        "--overwrite",
                        "--format",
                        "json",
                    ],
                    Some(workspace_root),
                )
                .await
            {
                Ok(output) if output.success => {
                    bytes = file_size(&output_path);
                    if bytes.is_none() {
                        status = "unknown".to_string();
                        error = Some(
                            "media-download succeeded but output file was not found".to_string(),
                        );
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

        if let Some(progress) = progress {
            progress.inc(1);
        }

        rows.push(MediaItem {
            doc_index: doc.index,
            doc_title: doc.title.clone(),
            doc_token: doc.token.clone(),
            doc_url: doc.url.clone(),
            token: item.token.clone(),
            tag: item.tag.clone(),
            mime: item.mime.clone(),
            name: item.name.clone(),
            href: item.href.clone(),
            output_path: Some(output_path.display().to_string()),
            bytes,
            status,
            error,
        });
    }

    let manifest_json = doc_dir_abs.join("media-manifest.json");
    let manifest_csv = doc_dir_abs.join("media-manifest.csv");
    let mut manifest_errors = Vec::new();
    if let Err(err) = write_json(&manifest_json, &rows) {
        manifest_errors.push(err.to_string());
    }
    if let Err(err) = write_csv(&manifest_csv, &rows) {
        manifest_errors.push(err.to_string());
    }

    let bytes = rows.iter().filter_map(|row| row.bytes).sum();
    let downloaded = rows.iter().filter(|row| row.bytes.is_some()).count();
    let row_errors = rows.iter().filter_map(|row| row.error.clone());
    let errors: Vec<_> = row_errors.chain(manifest_errors).collect();

    MediaSummary {
        count: rows.len(),
        downloaded,
        bytes: Some(bytes),
        dir: Some(doc_dir_abs.display().to_string()),
        error: if errors.is_empty() {
            None
        } else {
            Some(errors.join("\n"))
        },
    }
}

pub async fn fetch_doc_content(lark: &LarkCli, doc: &DocItem) -> Result<String> {
    let doc_ref = if doc.url.is_empty() {
        doc.token.as_str()
    } else {
        doc.url.as_str()
    };
    let parsed = lark
        .json_with_retry([
            "docs",
            "+fetch",
            "--api-version",
            "v2",
            "--as",
            "user",
            "--doc",
            doc_ref,
            "--doc-format",
            "markdown",
            "--scope",
            "full",
            "--format",
            "json",
        ])
        .await?;
    Ok(json_pointer_string(&parsed, "/data/document/content"))
}

/// Extract deduplicated media tags from doc markdown.
pub fn find_media_items(content: &str) -> Vec<MediaTag> {
    let mut seen: HashSet<(String, String)> = HashSet::new();
    let mut items = Vec::new();

    for captures in MEDIA_TAG_RE.captures_iter(content) {
        let tag = captures
            .get(1)
            .map(|value| value.as_str().to_ascii_lowercase())
            .unwrap_or_default();
        let attrs = captures
            .get(2)
            .map(|value| value.as_str())
            .unwrap_or_default();
        let mut token = None;
        let mut mime = None;
        let mut name = None;
        let mut href = None;

        for attr in MEDIA_ATTR_RE.captures_iter(attrs) {
            let key = attr
                .get(1)
                .map(|value| value.as_str())
                .unwrap_or_default()
                .to_ascii_lowercase();
            let value = attr
                .get(2)
                .map(|value| decode_html_entities(value.as_str()).to_string())
                .unwrap_or_default();
            match key.as_str() {
                "token" | "src" => token = Some(value),
                "mime" => mime = Some(value),
                "name" => name = Some(value),
                "href" | "url" => href = Some(value),
                _ => {}
            }
        }

        let Some(token) = token else { continue };
        if !seen.insert((tag.clone(), token.clone())) {
            continue;
        }
        items.push(MediaTag {
            token,
            tag,
            mime,
            name,
            href,
        });
    }

    items
}

#[cfg(test)]
mod tests {
    use super::find_media_items;

    #[test]
    fn extracts_image_and_source_tags() {
        let content = r#"
        <img token="abc" mime="image/png" name="a.png" />
        <source token="def" mime="video/mp4" name="v.mp4" />
        <image token="abc" />  <!-- dedup against img above? no, tag differs -->
        <file token="ghi" href="/x" />
        <whiteboard token="wb1" />
        <img token="abc" />  <!-- dup of first img, dropped -->
        "#;
        let items = find_media_items(content);
        // img abc, source def, image abc, file ghi, whiteboard wb1
        assert_eq!(items.len(), 5);
    }
}
