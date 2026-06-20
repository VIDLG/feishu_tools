//! Syntax highlighting for structured output (config TOML, JSON reports).
//!
//! Uses [`syntect-assets`] which bundles `bat`'s full curated syntax set
//! (~150 languages including TOML, JSON, YAML, Markdown, Rust, ...).
//!
//! Color policy follows the standard `--color=auto|always|never` convention
//! via `colorchoice` + `anstream`:
//! - `Auto`    (default): highlight only when stdout is a terminal
//! - `AlwaysAnsi` / `Always`: force highlighting even through `just`/`cargo run`
//! - `Never`:  never highlight
//!
//! `colorchoice_clap::Color::write_global()` in `main` populates the global
//! policy; we read it here via `ColorChoice::global()`.

use std::sync::{LazyLock, Mutex};

use anyhow::Result;
use colorchoice::ColorChoice;
use syntect::easy::HighlightLines;
use syntect::highlighting::Theme;
use syntect::util::as_24_bit_terminal_escaped;
use syntect_assets::assets::HighlightingAssets;

/// bat's full curated syntax/theme set, loaded once from the embedded
/// `syntaxes.bin` / `themes.bin` and shared across all calls.
///
/// `HighlightingAssets` itself is not `Sync` (it uses `once_cell::unsync`
/// internally), so we wrap it in a `Mutex` to make it shareable across
/// tokio worker threads. Highlighting is not a hot path — the lock cost
/// is negligible compared to the actual rendering work.
static ASSETS: LazyLock<Mutex<HighlightingAssets>> =
    LazyLock::new(|| Mutex::new(HighlightingAssets::from_binary()));

/// Default theme, picked to look reasonable on both light and dark terminals.
const DEFAULT_THEME: &str = "OneHalfDark";

/// Decide whether output should be colorized right now, reading the global
/// `colorchoice` policy. The policy is set once at startup from clap's
/// `--color` flag (see `colorchoice_clap::Color::write_global()`).
pub fn should_colorize() -> bool {
    match ColorChoice::global() {
        ColorChoice::Always | ColorChoice::AlwaysAnsi => true,
        ColorChoice::Never => false,
        ColorChoice::Auto => {
            // For Auto we delegate to anstream's TTY detection, which also
            // knows about NO_COLOR / CLICOLOR_FORCE env vars.
            anstream::AutoStream::choice(&std::io::stdout()) != ColorChoice::Never
        }
    }
}

/// Resolve a theme by name, falling back to [`DEFAULT_THEME`].
pub fn theme(name: Option<&str>) -> Theme {
    let assets = ASSETS.lock().expect("assets mutex not poisoned");
    assets.get_theme(name.unwrap_or(DEFAULT_THEME)).clone()
}

/// Highlight a multi-line string as the given syntax extension/token
/// (e.g. `"toml"`, `"json"`). Returns the original text unchanged when
/// colorization is disabled (per `should_colorize()`) or the syntax is
/// not recognized.
///
/// The caller writes the result through `anstream::print!` / `println!`,
/// which strips ANSI escapes automatically when stdout is not a terminal.
/// This is the "always emit, strip at the stream" pattern recommended by
/// the Rust CLI Working Group.
pub fn highlight(text: &str, syntax_name: &str, theme: &Theme) -> String {
    if !should_colorize() {
        return text.to_string();
    }

    let assets = ASSETS.lock().expect("assets mutex not poisoned");
    let syntax_set = assets
        .get_syntax_set()
        .expect("syntax set loads from binary");
    let Some(syntax) = syntax_set
        .find_syntax_by_extension(syntax_name)
        .or_else(|| syntax_set.find_syntax_by_token(syntax_name))
    else {
        return text.to_string();
    };

    let mut highlighter = HighlightLines::new(syntax, theme);
    // Pass `false` for `background` so we don't paint the whole line with
    // the theme's background color — that looks bad mixed with the user's
    // own terminal background. Foreground colors only.
    let mut out = String::with_capacity(text.len() + 256);
    for content in text.split_inclusive('\n') {
        match highlighter.highlight_line(content, syntax_set) {
            Ok(regions) => {
                out.push_str(&as_24_bit_terminal_escaped(&regions[..], false));
            }
            Err(_) => {
                out.push_str(content);
            }
        }
    }
    out
}

/// Highlight TOML text (used by `config show` / `config init`).
pub fn toml(text: &str) -> Result<String> {
    let theme = theme(None);
    Ok(highlight(text, "toml", &theme))
}

/// Highlight JSON text (used by pretty-printed JSON output, e.g. `quota`).
pub fn json(text: &str) -> Result<String> {
    let theme = theme(None);
    Ok(highlight(text, "json", &theme))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toml_highlighter_does_not_panic() {
        // Whether or not colorization is on, the function must not panic and
        // must preserve the visible text content.
        let text = "[storage]\nroot = \"x\"\n";
        let out = toml(text).unwrap();
        assert!(out.contains("storage"));
        assert!(out.contains("root"));
    }

    #[test]
    fn bat_assets_include_toml() {
        // Regression: the bundled default SyntaxSet from syntect itself
        // doesn't have TOML; syntect-assets (bat's set) does.
        let assets = ASSETS.lock().unwrap();
        let ss = assets.get_syntax_set().expect("syntax set loads");
        assert!(ss.find_syntax_by_extension("toml").is_some());
        assert!(ss.find_syntax_by_extension("json").is_some());
    }
}
