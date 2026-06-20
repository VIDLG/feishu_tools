use std::path::{Path, PathBuf};

use anstream::{eprintln, print, println};
use clap::{Args, CommandFactory, Parser, Subcommand};
use clap_complete::{Shell, generate};
use clap_verbosity_flag::Verbosity;
use tracing_subscriber::prelude::*;

mod commands;
mod concurrency;
mod config;
mod csvutil;
mod error;
mod highlight;
mod lark;
mod report;
mod util;

use commands::audit::AuditCommand;
use commands::backup::BackupCommand;
use commands::delete::DeleteCommand;
use commands::doctor::DoctorCommand;
use commands::list::ListCommand;
use commands::quota::QuotaCommand;
use commands::report::ReportCommand;
use config::{AppConfig, DEFAULT_CONFIG_PATH, expand_config_path, write_default_config};

#[derive(Debug, Parser)]
#[command(name = "fst")]
#[command(about = "Compact Feishu/Lark storage toolkit", version)]
struct Cli {
    /// Path to config.toml. Missing config falls back to built-in defaults.
    #[arg(long, global = true, default_value = DEFAULT_CONFIG_PATH)]
    config: PathBuf,

    /// When to colorize output (auto / always / never).
    /// `always` is useful under `just`/`cargo run`, where stdout is a pipe
    /// even though a real terminal is attached one level up.
    #[command(flatten)]
    color: colorchoice_clap::Color,

    /// Optional directory for daily-rotated log files (e.g. ./logs).
    /// When omitted, tracing only writes to stderr.
    #[arg(long, global = true)]
    log_file: Option<PathBuf>,

    #[command(flatten)]
    verbose: Verbosity,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Generate shell completion scripts.
    Completions { shell: Shell },

    /// Manage fst configuration.
    Config(ConfigCommand),

    /// List Feishu Drive resources without downloading them.
    List(ListCommand),

    /// Diagnose storage-heavy Feishu docs by exporting and sorting by size.
    Audit(AuditCommand),

    /// Export Feishu-native resources as local backups.
    Backup(BackupCommand),

    /// Build and apply human-confirmed delete plans.
    Delete(DeleteCommand),

    /// Show Drive quota details for a user.
    Quota(QuotaCommand),

    /// Summarize fst CSV reports.
    Report(ReportCommand),

    /// Check local fst/lark-cli configuration and auth status.
    Doctor(DoctorCommand),
}

#[derive(Debug, Args)]
struct ConfigCommand {
    #[command(subcommand)]
    command: Option<ConfigSubcommand>,
}

#[derive(Debug, Subcommand)]
enum ConfigSubcommand {
    /// Create a commented fst.toml in the current directory.
    Init {
        /// Overwrite existing config file.
        #[arg(long)]
        force: bool,
    },

    /// Print the effective configuration.
    Show,
}

fn init_tracing(verbose: &clap_verbosity_flag::Verbosity, log_file: Option<&Path>) {
    // clap-verbosity-flag gives us `-v` / `-vv` / `-vvv` / `-q` mapping to
    // Error/Warn/Info/Debug/Trace. Default (no flag) is Warn; we bump default
    // to Info so progress (search pages, exports) shows on stderr.
    //
    // `RUST_LOG` always wins. Progress goes to stderr via tracing, leaving
    // stdout clean for the command's product output (CSV/JSON paths,
    // summary tables, etc.).
    let default_level = verbose.tracing_level_filter();
    let env_or_default = format!("{}", default_level);
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(env_or_default));

    let stderr_layer = tracing_subscriber::fmt::layer()
        .with_target(false)
        .with_writer(std::io::stderr);

    // Optional file layer: appends to a daily-rotated file under the given dir.
    // Returned guard must live as long as the process; we leak it intentionally.
    let file_layer = log_file.map(|dir| {
        let appender = tracing_appender::rolling::daily(dir, "fst.log");
        tracing_subscriber::fmt::layer()
            .with_target(false)
            .with_ansi(false) // plain text in log files
            .with_writer(appender)
    });

    tracing_subscriber::registry()
        .with(filter)
        .with(stderr_layer)
        .with(file_layer)
        .init();
}

#[tokio::main]
async fn main() -> std::process::ExitCode {
    install_panic_hook();
    let cli = Cli::parse();
    init_tracing(&cli.verbose, cli.log_file.as_deref());
    cli.color.write_global();
    // Mirror the policy into `console` so doctor's manual styling (via
    // `console::style`) honors `--color` too.
    let choice = anstream::AutoStream::choice(&std::io::stdout());
    console::set_colors_enabled(!matches!(choice, colorchoice::ColorChoice::Never));

    let result = run(cli).await;
    match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(err) => {
            crate::error::report(&err);
            crate::error::exit_code_for(&err)
        }
    }
}

/// Install a panic hook that formats panics as one-line errors to stderr,
/// hiding the default std panic message in release builds. RUST_BACKTRACE
/// still works for diagnosis.
fn install_panic_hook() {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let payload = info.payload();
        let msg = if let Some(s) = payload.downcast_ref::<&'static str>() {
            (*s).to_string()
        } else if let Some(s) = payload.downcast_ref::<String>() {
            s.clone()
        } else {
            "Box<Any> panic".to_string()
        };
        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "<unknown>".to_string());
        eprintln!("fatal: internal panic at {location}");
        eprintln!("  {msg}");
        eprintln!();
        eprintln!(
            "This is a bug in fst. Please report it with: fst --version and the command you ran."
        );
        // Chain the previous hook so RUST_BACKTRACE=1 still prints backtraces.
        prev(info);
    }));
}

async fn run(cli: Cli) -> anyhow::Result<()> {
    match cli.command {
        Commands::Completions { shell } => {
            let mut command = Cli::command();
            let name = command.get_name().to_string();
            generate(shell, &mut command, name, &mut std::io::stdout());
            Ok(())
        }
        Commands::Config(command) => match command.command.unwrap_or(ConfigSubcommand::Show) {
            ConfigSubcommand::Init { force } => {
                let config_path = expand_config_path(&cli.config);
                write_default_config(&config_path, force)?;
                let display_path =
                    dunce::canonicalize(&config_path).unwrap_or_else(|_| config_path.clone());
                println!("Config written: {}", display_path.display());
                println!();
                // Preview the freshly written config (highlighted, like `config show`).
                let config = AppConfig::load(&cli.config)?;
                print_toml_highlighted(&config);
                Ok(())
            }
            ConfigSubcommand::Show => {
                let config_path = expand_config_path(&cli.config);
                let display_path = dunce::canonicalize(&config_path).unwrap_or(config_path);
                let source = if display_path.exists() {
                    "file"
                } else {
                    "defaults"
                };
                let config = AppConfig::load(&cli.config)?;
                println!("# Config path: {}", display_path.display());
                println!("# Source: {source}");
                println!();
                print_toml_highlighted(&config);
                Ok(())
            }
        },
        Commands::List(command) => {
            let config = AppConfig::load(&cli.config)?;
            command.run(&config).await
        }
        Commands::Audit(command) => {
            let config = AppConfig::load(&cli.config)?;
            command.run(&config).await
        }
        Commands::Backup(command) => {
            let config = AppConfig::load(&cli.config)?;
            command.run(&config).await
        }
        Commands::Delete(command) => {
            let config = AppConfig::load(&cli.config)?;
            command.run(&config).await
        }
        Commands::Quota(command) => command.run().await,
        Commands::Report(command) => command.run(),
        Commands::Doctor(command) => {
            let config_path = expand_config_path(&cli.config);
            let config = AppConfig::load(&config_path)?;
            command.run(&config, &config_path).await
        }
    }
}

/// Serialize config to pretty TOML, apply syntect highlighting when stdout
/// is a TTY, then print. Falls back to plain TOML when highlighting fails
/// or stdout is redirected (so the output stays machine-readable).
///
/// Shared by `config init` (preview freshly written file) and `config show`.
fn print_toml_highlighted(config: &AppConfig) {
    let toml = match toml_edit::ser::to_string_pretty(config) {
        Ok(t) => t,
        Err(err) => {
            eprintln!("warn: failed to serialize config preview: {err}");
            return;
        }
    };
    let rendered = crate::highlight::toml(&toml).unwrap_or(toml);
    print!("{rendered}");
}
