//! Shared audit helpers: CSV ingestion, Drive search, manifest building.

use std::ffi::OsString;
use std::path::Path;

use anyhow::{Context, Result};
use serde_json::Value;

use crate::csvutil;
use crate::lark::LarkCli;
use crate::util::{
    highlighted_title, json_optional_string, json_string, metadata_bytes, safe_file_part,
};

use super::model::{DocItem, EXPORTABLE_TYPES, RawDoc, export_extension};

/// Read fst list/audit CSV into [`RawDoc`] rows, optionally keeping only
/// exportable doc types.
pub fn read_raw_docs_from_csv(path: &Path, exportable_only: bool) -> Result<Vec<RawDoc>> {
    let rows = csvutil::read_csv_maps(path)?;
    let mut docs = Vec::new();

    for row in rows {
        let doc_type =
            csvutil::get(&row, &["doc_type", "type", "entity_type"]).to_ascii_lowercase();
        if exportable_only && !EXPORTABLE_TYPES.contains(&doc_type.as_str()) {
            continue;
        }
        docs.push(RawDoc {
            title: csvutil::get(&row, &["title"]),
            doc_type,
            token: csvutil::get(&row, &["token", "file_token"]),
            url: csvutil::get(&row, &["url"]),
            update_time_iso: csvutil::get_optional(&row, &["update_time_iso"]),
            create_time_iso: csvutil::get_optional(&row, &["create_time_iso"]),
            last_open_time_iso: csvutil::get_optional(&row, &["last_open_time_iso"]),
            entity_type: csvutil::get(&row, &["entity_type"]),
            metadata_bytes: csvutil::first_u64(
                &row,
                &["metadata_bytes", "total_bytes", "export_bytes", "bytes"],
            ),
        });
    }

    Ok(docs)
}

/// Page through `drive +search --as user --mine` and collect [`RawDoc`]s.
///
/// If `exportable_only` is true, non-exportable types (file, folder, wiki, ...)
/// are dropped, because audit/export cannot process them.
pub async fn search_docs(
    lark: &LarkCli,
    limit: usize,
    page_size: usize,
    exportable_only: bool,
) -> Result<Vec<RawDoc>> {
    tracing::info!(
        "Searching {}...",
        if exportable_only {
            "exportable docs"
        } else {
            "Drive resources"
        }
    );
    let mut docs = Vec::new();
    let mut page_token: Option<String> = None;
    let mut page = 0usize;

    loop {
        page += 1;
        let mut args: Vec<OsString> = vec![
            "drive".into(),
            "+search".into(),
            "--as".into(),
            "user".into(),
            "--mine".into(),
            "--query".into(),
            "".into(),
            "--page-size".into(),
            page_size.to_string().into(),
            "--format".into(),
            "json".into(),
        ];
        if let Some(token) = &page_token {
            args.push("--page-token".into());
            args.push(token.into());
        }

        let obj = lark
            .json_with_retry(args)
            .await
            .with_context(|| format!("drive +search page {page}"))?;
        let results = obj
            .pointer("/data/results")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();

        for result in results {
            if let Some(raw) = parse_search_result(&result, exportable_only) {
                docs.push(raw);
                if limit > 0 && docs.len() >= limit {
                    return Ok(docs);
                }
            }
        }

        tracing::info!("  page {page}: {} exportable docs", docs.len());
        let data = obj.get("data").unwrap_or(&Value::Null);
        if !data
            .get("has_more")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            return Ok(docs);
        }
        page_token = json_optional_string(data, "page_token");
        if page_token.is_none() {
            return Ok(docs);
        }
    }
}

fn parse_search_result(result: &Value, exportable_only: bool) -> Option<RawDoc> {
    let meta = result.get("result_meta").unwrap_or(&Value::Null);
    let doc_type = meta
        .get("doc_types")
        .or_else(|| result.get("entity_type"))
        .and_then(Value::as_str)?
        .to_ascii_lowercase();
    if exportable_only && !EXPORTABLE_TYPES.contains(&doc_type.as_str()) {
        return None;
    }

    let entity_type = json_string(result, "entity_type");
    let mut token = json_string(meta, "token");
    if entity_type == "WIKI"
        && let Some(icon_info) = meta.get("icon_info").and_then(Value::as_str)
        && let Ok(parsed) = serde_json::from_str::<Value>(icon_info)
        && let Some(wiki_token) = parsed.get("token").and_then(Value::as_str)
    {
        token = wiki_token.to_string();
    }

    let title = highlighted_title(result);

    Some(RawDoc {
        title,
        doc_type,
        token,
        url: json_string(meta, "url"),
        update_time_iso: json_optional_string(meta, "update_time_iso"),
        create_time_iso: json_optional_string(meta, "create_time_iso"),
        last_open_time_iso: json_optional_string(meta, "last_open_time_iso"),
        entity_type,
        metadata_bytes: metadata_bytes(meta),
    })
}

/// Build export-file names for each doc and wrap as [`DocItem`].
pub fn build_manifest(raw_docs: Vec<RawDoc>) -> Vec<DocItem> {
    raw_docs
        .into_iter()
        .enumerate()
        .map(|(offset, doc)| {
            let index = offset + 1;
            let ext = export_extension(&doc.doc_type);
            DocItem {
                export_file: format!(
                    "audit_{index:03}_{}.{}",
                    safe_file_part(&doc.title, 80),
                    ext
                ),
                index,
                title: doc.title,
                doc_type: doc.doc_type,
                token: doc.token,
                url: doc.url,
                update_time_iso: doc.update_time_iso,
                create_time_iso: doc.create_time_iso,
                last_open_time_iso: doc.last_open_time_iso,
                entity_type: doc.entity_type,
                expected_bytes: doc.metadata_bytes,
            }
        })
        .collect()
}
