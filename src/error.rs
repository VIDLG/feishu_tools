//! Top-level fst error type and process exit-code mapping.
//!
//! The fine-grained categorization lets scripts distinguish "auth failed,
//! re-login needed" (exit 3) from "transient network error" (exit 5), instead
//! of always seeing exit 1. This matches the conventions used by curl, ssh,
//! and other CLI tools that are commonly chained in shell scripts.
//!
//! Exit code map:
//! - 0  success
//! - 1  uncategorized / internal error (anyhow fallback)
//! - 2  usage error (bad args, missing config, invalid input)
//! - 3  auth failed (need re-login)
//! - 4  missing scope (need scope grant)
//! - 5  network/IO error (transient)
//! - 6  lark-cli invocation failure (non-transient subprocess error)
//!
//! The categorization is heuristic on top of `anyhow::Error`: we look for
//! `LarkError` in the cause chain and inspect string content. This is the
//! same heuristic `LarkError::is_auth_related` uses; centralized here so
//! exit codes stay consistent with retry/hint decisions.

use std::process::ExitCode;

use anstream::eprintln;

use crate::lark::LarkError;

/// Map an `anyhow::Error` to a process exit code by inspecting its cause
/// chain for known `LarkError` variants.
pub fn exit_code_for(err: &anyhow::Error) -> ExitCode {
    // Walk the cause chain looking for a typed LarkError.
    let mut current: Option<&(dyn std::error::Error + 'static)> = Some(err.as_ref());
    while let Some(e) = current {
        if let Some(lark_err) = e.downcast_ref::<LarkError>() {
            return match lark_err {
                LarkError::Auth(_) => ExitCategory::AuthFailed.exit_code(),
                LarkError::Spawn(_) => ExitCategory::Network.exit_code(),
                LarkError::SubprocessFailed(text) => {
                    let lower = text.to_ascii_lowercase();
                    // Reuse the same heuristic as is_auth_related so retry
                    // and exit-code decisions agree.
                    if lower.contains("permission") || lower.contains("scope") {
                        ExitCategory::MissingScope.exit_code()
                    } else if lower.contains("auth")
                        || lower.contains("login")
                        || lower.contains("unauthorized")
                    {
                        ExitCategory::AuthFailed.exit_code()
                    } else if lower.contains("timeout")
                        || lower.contains("connection")
                        || lower.contains("network")
                        || lower.contains("temporarily")
                    {
                        ExitCategory::Network.exit_code()
                    } else {
                        ExitCategory::LarkCliError.exit_code()
                    }
                }
                LarkError::JsonParse(_) | LarkError::NoJson => {
                    ExitCategory::LarkCliError.exit_code()
                }
            };
        }
        current = e.source();
    }

    // No typed LarkError found. Inspect the cause chain for known shapes:
    // fs errors carry their message in the underlying io::Error, not at top.
    let mut current: Option<&(dyn std::error::Error + 'static)> = Some(err.as_ref());
    while let Some(e) = current {
        let msg = e.to_string().to_ascii_lowercase();
        if msg.contains("no such file")
            || msg.contains("not found")
            || msg.contains("invalid")
            || msg.contains("parse")
            || msg.contains("missing")
            || msg.contains("系统找不到")
        // Windows: "system cannot find"
        {
            return ExitCategory::Usage.exit_code();
        }
        current = e.source();
    }
    ExitCategory::Internal.exit_code()
}

/// Pretty-print an error for top-level display. Includes the full cause chain
/// and any next-step hint. Goes to stderr (per UNIX convention).
pub fn report(err: &anyhow::Error) {
    // Walk the chain. Print the top message, then each cause indented.
    eprintln!("error: {}", err);
    let mut current = err.source();
    while let Some(cause) = current {
        let cause_str = cause.to_string();
        if !cause_str.is_empty() {
            eprintln!("  caused by: {cause_str}");
        }
        current = cause.source();
    }

    // If there's an auth/scope hint, surface it on its own line.
    let mut walker: Option<&(dyn std::error::Error + 'static)> = Some(err.as_ref());
    while let Some(e) = walker {
        if let Some(lark_err) = e.downcast_ref::<LarkError>()
            && let Some(hint) = lark_err.next_step_hint()
        {
            eprintln!();
            eprintln!("Next step: {hint}");
            break;
        }
        walker = e.source();
    }
}

/// Exit code categories. The numeric values are part of the public interface
/// — scripts depend on them — so they are explicitly numbered.
#[allow(dead_code)]
// `Success` is part of the documented public exit-code map
// even though the success path uses `ExitCode::SUCCESS` directly.
#[repr(u8)]
pub enum ExitCategory {
    Success = 0,
    Internal = 1,
    Usage = 2,
    AuthFailed = 3,
    MissingScope = 4,
    Network = 5,
    LarkCliError = 6,
}

impl ExitCategory {
    pub fn exit_code(self) -> ExitCode {
        ExitCode::from(self as u8)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_error_maps_to_exit_3() {
        let err: anyhow::Error = LarkError::Auth("401 unauthorized".into()).into();
        let code = exit_code_for(&err);
        assert_eq!(code, ExitCategory::AuthFailed.exit_code());
    }

    #[test]
    fn subprocess_with_scope_keyword_maps_to_exit_4() {
        let err: anyhow::Error =
            LarkError::SubprocessFailed("missing permission scope:drive:drive".into()).into();
        let code = exit_code_for(&err);
        assert_eq!(code, ExitCategory::MissingScope.exit_code());
    }

    #[test]
    fn subprocess_with_network_keyword_maps_to_exit_5() {
        let err: anyhow::Error =
            LarkError::SubprocessFailed("connection reset by peer".into()).into();
        let code = exit_code_for(&err);
        assert_eq!(code, ExitCategory::Network.exit_code());
    }

    #[test]
    fn generic_subprocess_maps_to_exit_6() {
        let err: anyhow::Error = LarkError::SubprocessFailed("doc not found".into()).into();
        let code = exit_code_for(&err);
        assert_eq!(code, ExitCategory::LarkCliError.exit_code());
    }

    #[test]
    fn anyhow_with_not_found_maps_to_usage_2() {
        // Mimic how anyhow + fs-err chain: outer "open CSV path" wraps
        // inner io::Error("no such file"). The classifier must walk the chain.
        let inner = std::io::Error::new(std::io::ErrorKind::NotFound, "no such file: config.toml");
        let err: anyhow::Error = anyhow::Error::new(inner).context("open CSV path");
        let code = exit_code_for(&err);
        assert_eq!(code, ExitCategory::Usage.exit_code());
    }

    #[test]
    fn anyhow_unrecognized_maps_to_internal_1() {
        let err: anyhow::Error = anyhow::anyhow!("unexpected runtime condition");
        let code = exit_code_for(&err);
        assert_eq!(code, ExitCategory::Internal.exit_code());
    }
}
