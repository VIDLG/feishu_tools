use std::path::Path;

use anstream::println;
use anyhow::Result;
use clap::Args;
use console::style;
use fs_err as fs;
use tabled::{Table, Tabled};

use crate::config::AppConfig;
use crate::lark::LarkCli;

/// Color a status token (ok / failed / skipped / warning). Respects the
/// `console` crate's TTY detection, so piping output is plain text.
fn color_status(status: &str) -> String {
    let styled = match status {
        "ok" | "success" => style(status).green().bold(),
        "failed" | "fail" | "error" => style(status).red().bold(),
        "skipped" | "skip" | "unknown" => style(status).yellow().bold(),
        "warning" | "warn" => style(status).yellow().bold(),
        _ => style(status),
    };
    styled.to_string()
}

/// Replace the trailing `: ok|failed|skipped|warning` token in a line with a
/// colored version. If the expected token is not present, returns the line
/// unchanged.
fn color_line_status(line: &str, status: &str) -> String {
    let suffix = format!(": {status}");
    line.replacen(&suffix, &format!(": {}", color_status(status)), 1)
}

#[derive(Tabled)]
struct ScopeRow {
    scope: &'static str,
    status: String,
}

#[derive(Debug, Args)]
pub struct DoctorCommand {
    /// Current user's user_id. Enables quota probe.
    #[arg(long)]
    user_id: Option<String>,

    /// A doc URL or token used to probe docs +fetch.
    #[arg(long)]
    doc: Option<String>,

    /// Document type for export probe: doc, docx, sheet, bitable, slides.
    #[arg(long)]
    doc_type: Option<String>,

    /// Document token for export probe. If omitted, --doc is used.
    #[arg(long)]
    doc_token: Option<String>,
}

impl DoctorCommand {
    pub async fn run(self, config: &AppConfig, config_path: &Path) -> Result<()> {
        println!("fst doctor");
        println!("config path        : {}", config_path.display());
        println!("storage root       : {}", config.storage.root.display());

        let lark = LarkCli::new();
        check_lark_path();
        check_lark_version(&lark).await;
        check_auth_status(&lark).await;

        println!();
        println!("API probes");
        probe_search(&lark).await;
        probe_root_folder_list(&lark).await;
        probe_auto_sample(&lark, config).await;
        if let Some(user_id) = self.user_id.as_deref() {
            probe_quota(&lark, user_id).await;
        } else {
            println!(
                "{}",
                color_line_status(
                    "probe quota        : skipped (--user-id not provided)",
                    "skipped"
                )
            );
        }
        if let Some(doc) = self.doc.as_deref() {
            probe_doc_fetch(&lark, doc).await;
        } else {
            println!(
                "{}",
                color_line_status(
                    "probe docs fetch   : skipped (--doc not provided)",
                    "skipped"
                )
            );
        }
        if let Some(doc_type) = self.doc_type.as_deref() {
            let token = self.doc_token.as_deref().or(self.doc.as_deref());
            if let Some(token) = token {
                probe_export(&lark, config, doc_type, token).await;
            } else {
                println!(
                    "{}",
                    color_line_status(
                        "probe export       : skipped (--doc-token or --doc not provided)",
                        "skipped"
                    )
                );
            }
        } else {
            println!(
                "{}",
                color_line_status(
                    "probe export       : skipped (--doc-type not provided)",
                    "skipped"
                )
            );
        }

        Ok(())
    }
}

fn check_lark_path() {
    match which::which("lark-cli") {
        Ok(path) => println!("lark-cli path      : {}", path.display()),
        Err(_) => println!(
            "{}",
            color_line_status("lark-cli path      : failed (not found in PATH)", "failed")
        ),
    }
}

async fn check_lark_version(lark: &LarkCli) {
    match lark.run(["--version"]).await {
        Ok(output) if output.success => {
            println!("lark-cli version   : {}", output.combined.trim())
        }
        Ok(output) => println!(
            "{}",
            color_line_status(
                &format!("lark-cli version   : failed\n{}", output.combined.trim()),
                "failed"
            )
        ),
        Err(err) => println!(
            "{}",
            color_line_status(
                &format!("lark-cli version   : failed to spawn: {err}"),
                "failed"
            )
        ),
    }
}

async fn check_auth_status(lark: &LarkCli) {
    match lark.run(["auth", "status"]).await {
        Ok(output) if output.success => {
            println!("{}", color_line_status("auth status        : ok", "ok"));
            let text = output.combined;
            let mut unverified = Vec::new();
            let rows = [
                "search:docs:read",
                "docs:document:export",
                "docx:document:readonly",
                "docs:document.media:download",
                "drive:drive:readonly",
                "drive:drive",
                "drive:quota_detail:read_one",
            ]
            .into_iter()
            .map(|scope| {
                let ok = text.contains(scope);
                if !ok {
                    unverified.push(scope);
                }
                ScopeRow {
                    scope,
                    status: if ok {
                        color_status("ok")
                    } else {
                        color_status("unknown")
                    },
                }
            })
            .collect::<Vec<_>>();
            println!("{}", Table::new(rows));
            if !unverified.is_empty() {
                println!("Authorize/check scopes:");
                println!("lark-cli auth login --scope \"{}\"", unverified.join(" "));
            }
        }
        Ok(output) => {
            println!(
                "{}",
                color_line_status("auth status        : failed", "failed")
            );
            println!("{}", output.combined.trim());
            println!("Try: lark-cli auth login --domain docs");
        }
        Err(err) => println!(
            "{}",
            color_line_status(
                &format!("auth status        : failed to spawn: {err}"),
                "failed"
            )
        ),
    }
}

async fn probe_search(lark: &LarkCli) {
    match lark
        .json([
            "drive",
            "+search",
            "--as",
            "user",
            "--query",
            "",
            "--page-size",
            "1",
            "--format",
            "json",
        ])
        .await
    {
        Ok(_) => println!("{}", color_line_status("probe search       : ok", "ok")),
        Err(err) => {
            println!(
                "{}",
                color_line_status("probe search       : failed", "failed")
            );
            println!("  {err}");
            println!("  Try: lark-cli auth login --scope \"search:docs:read\"");
        }
    }
}

async fn probe_root_folder_list(lark: &LarkCli) {
    let params = serde_json::json!({ "folder_token": "", "page_size": 20 }).to_string();
    match lark
        .json([
            "drive",
            "files",
            "list",
            "--as",
            "user",
            "--params",
            params.as_str(),
            "--format",
            "json",
        ])
        .await
    {
        Ok(_) => println!("{}", color_line_status("probe root list    : ok", "ok")),
        Err(err) => {
            println!(
                "{}",
                color_line_status("probe root list    : failed", "failed")
            );
            println!("  {err}");
            println!("  Try: lark-cli auth login --scope \"drive:drive:readonly\"");
        }
    }
}

async fn probe_auto_sample(lark: &LarkCli, config: &AppConfig) {
    let sample = match lark
        .json([
            "drive",
            "+search",
            "--as",
            "user",
            "--query",
            "",
            "--mine",
            "--doc-types",
            "docx,doc,sheet,bitable,slides",
            "--page-size",
            "1",
            "--format",
            "json",
        ])
        .await
    {
        Ok(obj) => obj
            .pointer("/data/results")
            .and_then(|value| value.as_array())
            .and_then(|items| items.first())
            .cloned(),
        Err(err) => {
            println!(
                "{}",
                color_line_status("probe sample search: failed", "failed")
            );
            println!("  {err}");
            return;
        }
    };

    let Some(sample) = sample else {
        println!(
            "{}",
            color_line_status(
                "probe sample       : skipped (no exportable owned doc found)",
                "skipped"
            )
        );
        return;
    };
    let meta = sample.get("result_meta").unwrap_or(&sample);
    let doc_type = meta
        .get("doc_types")
        .or_else(|| sample.get("entity_type"))
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let mut token = meta
        .get("token")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .to_string();
    if sample.get("entity_type").and_then(|value| value.as_str()) == Some("WIKI")
        && let Some(wiki_token) = wiki_inner_token(meta)
    {
        token = wiki_token;
    }
    let url = meta
        .get("url")
        .and_then(|value| value.as_str())
        .unwrap_or(&token);

    if doc_type == "docx" || doc_type == "doc" {
        probe_doc_fetch(lark, url).await;
    } else {
        println!(
            "{}",
            color_line_status(
                &format!("probe docs fetch   : skipped (sample is {doc_type})"),
                "skipped"
            )
        );
    }
    if !doc_type.is_empty() && !token.is_empty() {
        probe_export(lark, config, &doc_type, &token).await;
    } else {
        println!(
            "{}",
            color_line_status(
                "probe export       : skipped (sample missing type/token)",
                "skipped"
            )
        );
    }
}

fn wiki_inner_token(meta: &serde_json::Value) -> Option<String> {
    let icon_info = meta.get("icon_info")?.as_str()?;
    let parsed = serde_json::from_str::<serde_json::Value>(icon_info).ok()?;
    parsed
        .get("token")
        .and_then(|value| value.as_str())
        .map(ToOwned::to_owned)
}

async fn probe_quota(lark: &LarkCli, user_id: &str) {
    let params = serde_json::json!({ "quota_detail_id": user_id }).to_string();
    match lark
        .json([
            "drive",
            "quota_details",
            "get",
            "--as",
            "user",
            "--params",
            params.as_str(),
            "--format",
            "json",
        ])
        .await
    {
        Ok(_) => println!("{}", color_line_status("probe quota        : ok", "ok")),
        Err(err) => {
            println!(
                "{}",
                color_line_status("probe quota        : failed", "failed")
            );
            println!("  {err}");
            println!("  Try: lark-cli auth login --scope \"drive:quota_detail:read_one\"");
        }
    }
}

async fn probe_doc_fetch(lark: &LarkCli, doc: &str) {
    match lark
        .json([
            "docs",
            "+fetch",
            "--api-version",
            "v2",
            "--as",
            "user",
            "--doc",
            doc,
            "--doc-format",
            "markdown",
            "--scope",
            "full",
            "--format",
            "json",
        ])
        .await
    {
        Ok(_) => println!("{}", color_line_status("probe docs fetch   : ok", "ok")),
        Err(err) => {
            println!(
                "{}",
                color_line_status("probe docs fetch   : failed", "failed")
            );
            println!("  {err}");
            println!("  Try: lark-cli auth login --scope \"docx:document:readonly\"");
        }
    }
}

async fn probe_export(lark: &LarkCli, config: &AppConfig, doc_type: &str, token: &str) {
    let workspace_root = config.workspace_root();
    if let Err(err) = fs::create_dir_all(&workspace_root) {
        println!(
            "{}",
            color_line_status(
                &format!("probe export       : failed to create workspace root: {err}"),
                "failed"
            )
        );
        return;
    }
    let temp_dir = match tempfile::Builder::new()
        .prefix("doctor-probe-")
        .tempdir_in(&workspace_root)
    {
        Ok(dir) => dir,
        Err(err) => {
            println!(
                "{}",
                color_line_status(
                    &format!("probe export       : failed to create temp probe dir: {err}"),
                    "failed"
                )
            );
            return;
        }
    };
    let probe_dir_abs = temp_dir.path().to_path_buf();

    let extension = match doc_type {
        "sheet" => "xlsx",
        "bitable" => "base",
        "slides" => "pptx",
        _ => "docx",
    };
    let output = lark
        .run_in(
            [
                "drive",
                "+export",
                "--as",
                "user",
                "--doc-type",
                doc_type,
                "--token",
                token,
                "--file-extension",
                extension,
                "--file-name",
                "doctor-probe",
                "--output-dir",
                ".",
                "--overwrite",
            ],
            Some(&probe_dir_abs),
        )
        .await;

    match output {
        Ok(output) if output.success => {
            println!("{}", color_line_status("probe export       : ok", "ok"))
        }
        Ok(output) => {
            println!(
                "{}",
                color_line_status("probe export       : failed", "failed")
            );
            println!("  {}", output.combined.trim());
            println!("  Try: lark-cli auth login --scope \"docs:document:export\"");
        }
        Err(err) => println!(
            "{}",
            color_line_status(
                &format!("probe export       : failed to spawn: {err}"),
                "failed"
            )
        ),
    }
}
