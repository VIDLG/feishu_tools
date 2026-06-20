use fs_err as fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

pub const DEFAULT_CONFIG_PATH: &str = "~/.fst/config.toml";
pub const REPORTS_DIR: &str = "reports";
pub const EXPORTS_DIR: &str = "backups/exports";
pub const MEDIA_DIR: &str = "backups/media";
pub const FILES_DIR: &str = "backups/files";
const DEFAULT_CONFIG_TEMPLATE: &str = include_str!("config.example.toml");

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AppConfig {
    #[serde(default)]
    pub storage: StorageConfig,
    #[serde(default)]
    pub list: ListConfig,
    #[serde(default)]
    pub audit: AuditConfig,
    #[serde(default)]
    pub delete: DeleteConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageConfig {
    #[serde(default = "default_storage_root")]
    pub root: PathBuf,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            root: default_storage_root(),
        }
    }
}

/// Compute a platform-appropriate default storage root.
///
/// Config (`~/.fst/config.toml`) and storage (`~/.fst/storage/`) share the
/// same `~/.fst/` home so users can find, back up, or migrate everything
/// in one place. Mirrors the convention used by git (`~/.gitconfig` +
/// `~/.git/`), cargo (`~/.cargo/`), rustup (`~/.rustup/`).
///
/// - Linux/macOS: ~/.fst/storage
/// - Windows:     %USERPROFILE%\.fst\storage
///
/// Falls back to `./fst-storage` if home can't be resolved (rare; only in
/// headless containers without HOME/USERPROFILE).
pub fn default_storage_root() -> PathBuf {
    dirs::home_dir()
        .map(|home| home.join(".fst").join("storage"))
        .unwrap_or_else(|| PathBuf::from("fst-storage"))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListConfig {
    pub page_size: usize,
    pub query: String,
    pub mine: bool,
}

impl Default for ListConfig {
    fn default() -> Self {
        Self {
            page_size: 20,
            query: String::new(),
            mine: true,
        }
    }
}

/// Audit command configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditConfig {
    pub top_limit: usize,
    pub include_media: bool,
    /// Max number of concurrent lark-cli exports/media-downloads.
    /// 1 reproduces the old serial behavior.
    pub concurrency: usize,
}

impl Default for AuditConfig {
    fn default() -> Self {
        Self {
            top_limit: 100,
            include_media: true,
            concurrency: 4,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeleteConfig {
    pub min_mb: f64,
    pub top: usize,
    pub include_unknown: bool,
}

impl Default for DeleteConfig {
    fn default() -> Self {
        Self {
            min_mb: 100.0,
            top: 0,
            include_unknown: true,
        }
    }
}

pub fn expand_config_path(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    let expanded = if text == "~" {
        home_dir().unwrap_or_else(|| path.to_path_buf())
    } else if let Some(rest) = text.strip_prefix("~/").or_else(|| text.strip_prefix("~\\")) {
        home_dir()
            .map(|home| home.join(rest))
            .unwrap_or_else(|| path.to_path_buf())
    } else {
        path.to_path_buf()
    };

    if expanded.is_absolute() {
        expanded
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(&expanded))
            .unwrap_or(expanded)
    }
}

fn home_dir() -> Option<PathBuf> {
    dirs::home_dir()
}

impl AppConfig {
    pub fn load(path: &Path) -> Result<Self> {
        let path = expand_config_path(path);
        if !path.exists() {
            return Ok(Self::default());
        }
        let text =
            fs::read_to_string(&path).with_context(|| format!("read config {}", path.display()))?;
        let config = toml_edit::de::from_str(&text)
            .with_context(|| format!("parse config {}", path.display()))?;
        Ok(config)
    }

    pub fn workspace_root(&self) -> PathBuf {
        self.storage.root.clone()
    }

    pub fn reports_dir(&self) -> PathBuf {
        self.workspace_root().join(REPORTS_DIR)
    }
}

pub fn write_default_config(path: &Path, force: bool) -> Result<()> {
    let path = expand_config_path(path);
    if path.exists() && !force {
        bail!(
            "config already exists: {} (use --force to overwrite)",
            path.display()
        );
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create config dir {}", parent.display()))?;
    }
    fs::write(&path, DEFAULT_CONFIG_TEMPLATE)
        .with_context(|| format!("write config {}", path.display()))
}
