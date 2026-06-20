use std::collections::VecDeque;
use std::ffi::OsString;

use anstream::println;
use anyhow::{Context, Result};
use clap::Args;

use serde::Serialize;
use serde_json::Value;

use crate::config::AppConfig;
use crate::lark::LarkCli;
use crate::report::{write_csv, write_json};
use crate::util::{
    highlighted_title, json_optional_string, json_string, json_string_any, metadata_bytes,
    timestamp_for_filename,
};
use fs_err as fs;

#[derive(Debug, Args)]
pub struct ListCommand {
    /// Search query. Empty means list by filters only.
    #[arg(long)]
    query: Option<String>,

    /// Only list resources owned by current user. Overrides [list].mine.
    #[arg(long)]
    mine: Option<bool>,

    /// Comma-separated doc types, e.g. docx,sheet,bitable,file,folder,wiki,slides.
    #[arg(long)]
    doc_types: Option<String>,

    /// Only process the first N resources. 0 means all pages.
    #[arg(long, default_value_t = 0)]
    limit: usize,

    /// Drive folder token to list using drive files list. Empty string means Drive root.
    #[arg(long)]
    folder_token: Option<String>,

    /// Recursively traverse folders when --folder-token is used.
    #[arg(long)]
    recursive: bool,

    /// Safety cap for recursive folder listing. 0 means no explicit cap.
    #[arg(long, default_value_t = 0)]
    max_items: usize,

    /// Search page size. Overrides [list].page_size.
    #[arg(long)]
    page_size: Option<usize>,
}

impl ListCommand {
    pub async fn run(self, config: &AppConfig) -> Result<()> {
        let reports_dir = config.reports_dir();
        fs::create_dir_all(&reports_dir)?;
        let timestamp = timestamp_for_filename();
        let json_path = reports_dir.join(format!("list-{timestamp}.json"));
        let csv_path = reports_dir.join(format!("list-{timestamp}.csv"));

        let lark = LarkCli::new();
        let query = self.query.unwrap_or_else(|| config.list.query.clone());
        let mine = self.mine.unwrap_or(config.list.mine);
        let page_size = self.page_size.unwrap_or(config.list.page_size);

        let rows = if let Some(folder_token) = self.folder_token.as_deref() {
            let cap = match (self.limit, self.max_items) {
                (0, 0) => 0,
                (0, max_items) => max_items,
                (limit, 0) => limit,
                (limit, max_items) => limit.min(max_items),
            };
            list_folder_tree(&lark, folder_token, self.recursive, cap, page_size).await?
        } else {
            search_all(
                &lark,
                &query,
                mine,
                self.doc_types.as_deref(),
                self.limit,
                page_size,
            )
            .await?
        };
        write_json(&json_path, &rows)?;
        write_csv(&csv_path, &rows)?;

        println!("Listed {} resources", rows.len());
        println!("JSON: {}", json_path.display());
        println!("CSV : {}", csv_path.display());
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ListedResource {
    pub index: usize,
    pub title: String,
    pub doc_type: String,
    pub entity_type: String,
    pub token: String,
    pub url: String,
    pub path: String,
    pub update_time_iso: Option<String>,
    pub create_time_iso: Option<String>,
    pub last_open_time_iso: Option<String>,
    pub metadata_bytes: Option<u64>,
    pub size_source: String,
}

async fn search_all(
    lark: &LarkCli,
    query: &str,
    mine: bool,
    doc_types: Option<&str>,
    limit: usize,
    page_size: usize,
) -> Result<Vec<ListedResource>> {
    let mut rows = Vec::new();
    let mut page_token: Option<String> = None;
    let mut page = 0usize;

    loop {
        page += 1;
        let mut args: Vec<OsString> = vec![
            "drive".into(),
            "+search".into(),
            "--as".into(),
            "user".into(),
            "--query".into(),
            query.into(),
            "--page-size".into(),
            page_size.to_string().into(),
            "--format".into(),
            "json".into(),
        ];
        if mine {
            args.push("--mine".into());
        }
        if let Some(doc_types) = doc_types.filter(|value| !value.trim().is_empty()) {
            args.push("--doc-types".into());
            args.push(doc_types.into());
        }
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
            let row = parse_result(rows.len() + 1, &result);
            rows.push(row);
            if limit > 0 && rows.len() >= limit {
                return Ok(rows);
            }
        }

        tracing::info!("page {page}: {} resources", rows.len());
        let data = obj.get("data").unwrap_or(&Value::Null);
        if !data
            .get("has_more")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            return Ok(rows);
        }
        page_token = json_optional_string(data, "page_token");
        if page_token.is_none() {
            return Ok(rows);
        }
    }
}

async fn list_folder_tree(
    lark: &LarkCli,
    root_folder_token: &str,
    recursive: bool,
    max_items: usize,
    page_size: usize,
) -> Result<Vec<ListedResource>> {
    let mut rows = Vec::new();
    let mut queue = VecDeque::new();
    queue.push_back((root_folder_token.to_string(), String::new()));

    while let Some((folder_token, folder_path)) = queue.pop_front() {
        let mut page_token: Option<String> = None;
        loop {
            let params = folder_list_params(&folder_token, page_size, page_token.as_deref());
            let obj = lark
                .json_with_retry([
                    "drive",
                    "files",
                    "list",
                    "--params",
                    params.as_str(),
                    "--format",
                    "json",
                ])
                .await
                .with_context(|| format!("drive files list folder {folder_token}"))?;
            let files = obj
                .pointer("/data/files")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();

            for file in files {
                let index = rows.len() + 1;
                let row = parse_file_list_item(index, &folder_path, &file);
                if recursive && row.doc_type == "folder" && !row.token.is_empty() {
                    queue.push_back((row.token.clone(), row.path.clone()));
                }
                rows.push(row);
                if max_items > 0 && rows.len() >= max_items {
                    return Ok(rows);
                }
            }

            let data = obj.get("data").unwrap_or(&Value::Null);
            if !data
                .get("has_more")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                break;
            }
            page_token = json_optional_string(data, "next_page_token");
            if page_token.is_none() {
                break;
            }
        }

        if !recursive {
            break;
        }
    }

    Ok(rows)
}

fn folder_list_params(folder_token: &str, page_size: usize, page_token: Option<&str>) -> String {
    let mut params = serde_json::json!({
        "folder_token": folder_token,
        "page_size": page_size,
    });
    if let Some(page_token) = page_token {
        params["page_token"] = Value::String(page_token.to_string());
    }
    params.to_string()
}

fn parse_file_list_item(index: usize, parent_path: &str, file: &Value) -> ListedResource {
    let title = json_string_any(file, &["name", "title"]);
    let doc_type = file
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_ascii_lowercase();
    let token = json_string_any(file, &["token", "file_token"]);
    let path = if parent_path.is_empty() {
        title.clone()
    } else {
        format!("{parent_path}/{title}")
    };
    let metadata_bytes = metadata_bytes(file);

    ListedResource {
        index,
        title,
        doc_type: doc_type.clone(),
        entity_type: doc_type,
        token,
        url: json_string(file, "url"),
        path,
        update_time_iso: json_optional_string(file, "modified_time_iso")
            .or_else(|| json_optional_string(file, "update_time_iso")),
        create_time_iso: json_optional_string(file, "created_time_iso")
            .or_else(|| json_optional_string(file, "create_time_iso")),
        last_open_time_iso: None,
        metadata_bytes,
        size_source: size_source(metadata_bytes),
    }
}

fn parse_result(index: usize, result: &Value) -> ListedResource {
    let meta = result.get("result_meta").unwrap_or(&Value::Null);
    let doc_type = meta
        .get("doc_types")
        .or_else(|| result.get("entity_type"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_ascii_lowercase();
    let entity_type = json_string(result, "entity_type");
    let title = highlighted_title(result);
    let token = json_string(meta, "token");

    let metadata_bytes = metadata_bytes(meta);

    ListedResource {
        index,
        title,
        doc_type,
        entity_type,
        token,
        url: json_string(meta, "url"),
        path: String::new(),
        update_time_iso: json_optional_string(meta, "update_time_iso"),
        create_time_iso: json_optional_string(meta, "create_time_iso"),
        last_open_time_iso: json_optional_string(meta, "last_open_time_iso"),
        metadata_bytes,
        size_source: size_source(metadata_bytes),
    }
}

fn size_source(bytes: Option<u64>) -> String {
    if bytes.is_some() {
        "metadata"
    } else {
        "unknown"
    }
    .to_string()
}
