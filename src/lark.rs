use std::ffi::{OsStr, OsString};
use std::path::Path;

use anyhow::{Context, Result};
use serde_json::Value;
use thiserror::Error;
use tokio::process::Command;
use tokio_retry::RetryIf;
use tokio_retry::strategy::{ExponentialBackoff, jitter};
use tracing::debug;

/// Typed errors returned by lark-cli calls. Lower-level than `anyhow::Error`
/// so callers can match on specific failure modes (auth → prompt re-login,
/// scope → prompt scope grant, transient → retry).
#[derive(Debug, Error)]
pub enum LarkError {
    #[error("spawn lark-cli failed: {0}")]
    Spawn(#[from] std::io::Error),

    #[error("lark-cli auth status failed:\n{0}")]
    Auth(String),

    #[error("lark-cli failed (exit non-zero):\n{0}")]
    SubprocessFailed(String),

    #[error("lark-cli output JSON parse failed: {0}")]
    JsonParse(#[from] serde_json::Error),

    #[error("no JSON object found in lark-cli output")]
    NoJson,
}

impl LarkError {
    /// Heuristic: does this error look like an auth/scope issue? Used by
    /// [`retry_transient`] to fail-fast instead of retrying auth failures
    /// (which can never succeed without user action).
    pub fn is_auth_related(&self) -> bool {
        let text = match self {
            LarkError::Auth(t) | LarkError::SubprocessFailed(t) => t,
            _ => return false,
        };
        let lower = text.to_ascii_lowercase();
        lower.contains("auth")
            || lower.contains("login")
            || lower.contains("unauthorized")
            || lower.contains("permission")
            || lower.contains("scope")
    }

    /// Suggested next step the user can paste into the shell. Returned for
    /// auth/scope errors; `None` for everything else.
    pub fn next_step_hint(&self) -> Option<String> {
        let text = match self {
            LarkError::Auth(t) | LarkError::SubprocessFailed(t) => t,
            _ => return None,
        };
        let lower = text.to_ascii_lowercase();
        if lower.contains("auth") || lower.contains("login") || lower.contains("unauthorized") {
            Some(
                "run `lark-cli auth login --domain docs` or use the `auth login --scope ...` hint above"
                    .to_string(),
            )
        } else if lower.contains("permission") || lower.contains("scope") {
            Some(
                "grant the missing scope in the developer console, then run `lark-cli auth login --scope \"<missing_scope>\"` as user"
                    .to_string(),
            )
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct LarkCli;

impl LarkCli {
    pub fn new() -> Self {
        Self
    }

    /// Run lark-cli with default cwd.
    pub async fn run<I, S>(&self, args: I) -> Result<LarkOutput>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.run_in(args, None).await
    }

    /// Like [`Self::run`] but retries transient failures. Convenience wrapper
    /// over [`Self::run_in_with_retry`] for callers that don't need a cwd.
    pub async fn run_with_retry<I, S>(&self, args: I) -> Result<LarkOutput>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.run_in_with_retry(args, None).await
    }

    /// Run lark-cli inside `cwd`. lark-cli refuses absolute `--output` paths
    /// (path-traversal guard), so callers that want to write to a specific
    /// directory should pass it here and use relative `--output` values,
    /// instead of using `std::env::set_current_dir` which would race under
    /// concurrent callers.
    pub async fn run_in<I, S>(&self, args: I, cwd: Option<&Path>) -> Result<LarkOutput>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let args: Vec<OsString> = args.into_iter().map(|a| a.as_ref().to_owned()).collect();
        let program = which::which("lark-cli").unwrap_or_else(|_| "lark-cli".into());
        let mut command = Command::new(program);
        if let Some(cwd) = cwd {
            command.current_dir(cwd);
        }
        command.args(args.iter());
        debug!(?command, "running lark-cli");
        let output = command.output().await.context("spawn lark-cli")?;
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        let combined = format!("{stdout}{stderr}");

        Ok(LarkOutput {
            success: output.status.success(),
            stdout,
            combined,
        })
    }

    pub async fn json<I, S>(&self, args: I) -> Result<Value>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let output = self.run(args).await?;
        if !output.success {
            return Err(LarkError::SubprocessFailed(output.combined.trim().to_string()).into());
        }
        output.parse_json()
    }

    /// Like [`Self::json`] but retries transient failures with exponential
    /// backoff. Auth/scope errors are NOT retried (no point — the user has to
    /// act first). Defaults: 3 attempts, starting at 500ms, jittered.
    pub async fn json_with_retry<I, S>(&self, args: I) -> Result<Value>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let args: Vec<OsString> = args.into_iter().map(|a| a.as_ref().to_owned()).collect();
        retry_transient_skip_auth(|| {
            let args = args.clone();
            async move { self.json(args).await }
        })
        .await
    }

    /// Like [`Self::run_in`] but retries transient failures with exponential
    /// backoff. Used by export/download workers (large lists are likely to hit
    /// occasional network/rate-limit hiccups).
    pub async fn run_in_with_retry<I, S>(&self, args: I, cwd: Option<&Path>) -> Result<LarkOutput>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let args: Vec<OsString> = args.into_iter().map(|a| a.as_ref().to_owned()).collect();
        let cwd = cwd.map(|p| p.to_path_buf());
        retry_transient_skip_auth(move || {
            let args = args.clone();
            let cwd = cwd.clone();
            async move { self.run_in(args, cwd.as_deref()).await }
        })
        .await
    }
}

/// Run an lark-cli call under exponential backoff, retrying transient errors.
///
/// - Auth/scope errors (per [`LarkError::is_auth_related`]) fail fast — retrying
///   cannot help until the user grants access.
/// - Spawn errors and subprocess failures are retried up to 3 times with
///   jittered exponential backoff (500ms → 1s → 2s).
///
/// The strategy is shared between `LarkCli::json_with_retry` and
/// `run_in_with_retry` so that retry semantics stay consistent.
async fn retry_transient_skip_auth<F, Fut, T>(f: F) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T>>,
{
    let strategy = ExponentialBackoff::from_millis(500)
        .max_delay(std::time::Duration::from_secs(5))
        .factor(2)
        .map(jitter)
        .take(2);

    RetryIf::start(strategy, f, |err: &anyhow::Error| {
        if let Some(lark_err) = err.downcast_ref::<LarkError>()
            && lark_err.is_auth_related()
        {
            tracing::debug!(error = %lark_err, "auth-related error, not retrying");
            return false;
        }
        tracing::warn!(error = %err, "transient lark-cli error, retrying");
        true
    })
    .await
}

/// Decorate a captured lark-cli failure string with a next-step hint when
/// the text looks auth/scope-related. Used by export/download workers when
/// recording row-level errors.
pub fn append_hint(message: String) -> String {
    let err = LarkError::SubprocessFailed(message.clone());
    match err.next_step_hint() {
        Some(hint) => format!("{message}\n\nNext step: {hint}"),
        None => message,
    }
}

/// Format an `anyhow::Error` returned from a retrying lark-cli call.
///
/// Walks the full cause chain so the user sees, e.g.,
/// "drive +search page 3"
///   → "lark-cli failed (exit non-zero): HTTP 429"
/// And if the underlying error is a [`LarkError`] with an auth/scope hint,
/// appends "Next step: ...".
pub fn format_error_with_hint(err: &anyhow::Error) -> String {
    let mut message = err.to_string();
    let mut current = err.source();
    while let Some(cause) = current {
        let cause_str = cause.to_string();
        // Avoid duplicating identical strings (anyhow often wraps
        // contextually without changing the message).
        if !message.contains(&cause_str) && !cause_str.is_empty() {
            message.push_str("\n  caused by: ");
            message.push_str(&cause_str);
        }
        current = cause.source();
    }
    if let Some(lark_err) = err.downcast_ref::<LarkError>()
        && let Some(hint) = lark_err.next_step_hint()
    {
        message.push_str("\n\nNext step: ");
        message.push_str(&hint);
    }
    message
}

/// Check `lark-cli auth status` and warn about missing common scopes.
///
/// Shared by `audit`, `backup`, and any command that needs authenticated
/// lark-cli access. Failure of the auth status command is fatal; missing
/// scopes are only a warning.
pub async fn check_auth(lark: &LarkCli, required: &[&str]) -> Result<()> {
    tracing::info!("Checking lark-cli auth status...");
    let output = lark.run(["auth", "status"]).await?;
    if !output.success {
        return Err(LarkError::Auth(output.combined.trim().to_string()).into());
    }

    let missing: Vec<_> = required
        .iter()
        .copied()
        .filter(|scope| !output.combined.contains(*scope))
        .collect();
    if !missing.is_empty() {
        tracing::warn!("auth status output did not mention these useful scopes:");
        for scope in &missing {
            tracing::warn!("  - {scope}");
        }
        tracing::warn!("Try: lark-cli auth login --domain docs");
    }
    Ok(())
}

#[derive(Debug)]
pub struct LarkOutput {
    pub success: bool,
    pub stdout: String,
    pub combined: String,
}

impl LarkOutput {
    pub fn parse_json(&self) -> Result<Value> {
        parse_json_from_text(&self.stdout)
            .or_else(|_| parse_json_from_text(&self.combined))
            .map_err(Into::into)
    }
}

pub fn parse_json_from_text(text: &str) -> std::result::Result<Value, LarkError> {
    let start = text.find('{').ok_or(LarkError::NoJson)?;
    let end = text.rfind('}').ok_or(LarkError::NoJson)?;
    let json = &text[start..=end];
    // Use serde_path_to_error to get JSON pointer-style location on failure.
    let mut deserializer = serde_json::Deserializer::from_str(json);
    let value: Value = serde_path_to_error::deserialize(&mut deserializer)
        .map_err(|e| LarkError::JsonParse(e.into_inner()))?;
    deserializer.end()?;
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_json_embedded_in_cli_noise() {
        let value = parse_json_from_text("notice\n{\"ok\":true}\nwarning").unwrap();
        assert_eq!(value["ok"], true);
    }

    #[test]
    fn no_json_yields_typed_error() {
        let err = parse_json_from_text("just plain text").unwrap_err();
        assert!(matches!(err, LarkError::NoJson));
    }

    #[test]
    fn auth_related_hint_detection() {
        let err = LarkError::SubprocessFailed("HTTP 401 unauthorized".into());
        assert!(err.is_auth_related());
        assert!(err.next_step_hint().is_some());

        let err = LarkError::SubprocessFailed("network timeout".into());
        assert!(!err.is_auth_related());
        assert!(err.next_step_hint().is_none());
    }
}
