mod clipboard;
mod config;
mod diff;
mod doctor;
mod embedded;
mod source;
mod store;
mod syntax;
mod theme;
mod ui;

use std::{path::PathBuf, process::ExitCode};

use anyhow::Result;
use clap::{Parser, Subcommand};
use etcetera::BaseStrategy;

use source::Source;

/// A terminal code reviewer for work done by coding agents.
///
/// Plain `glos` reviews the current git repo: the branch since it left its base, uncommitted and
/// untracked files included.
#[derive(Parser)]
#[command(version)]
struct Cli {
    /// The database to use instead of `$XDG_DATA_HOME/glos/glos.db`.
    #[arg(long, env = "GLOS_DB", global = true)]
    db: Option<PathBuf>,

    /// More detail in the log (`~/.local/state/glos/glos.log`); repeat for more.
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    verbose: u8,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Review two files or two folders, like `git diff --no-index`.
    Diff { old: PathBuf, new: PathBuf },
    /// Check glos's setup, like `hx --health`.
    Doctor,
}

/// XDG folders on macOS too, as Helix does.
fn default_db() -> Result<PathBuf> {
    Ok(etcetera::base_strategy::Xdg::new()?
        .data_dir()
        .join("glos/glos.db"))
}

/// When glos started, for the first-screen time in the log (target: under 100 ms).
pub static STARTED: std::sync::LazyLock<std::time::Instant> =
    std::sync::LazyLock::new(std::time::Instant::now);

fn main() -> ExitCode {
    std::sync::LazyLock::force(&STARTED);
    match run() {
        Ok(code) => code,
        Err(e) => {
            eprintln!("glos: {e:#}");
            ExitCode::FAILURE
        }
    }
}

/// Logs never go to the terminal: in the UI they'd corrupt the screen, and in `glos mcp`
/// stdout carries the protocol. They go to `$XDG_STATE_HOME/glos/glos.log`.
fn init_log(verbose: u8) -> Result<PathBuf> {
    let path = etcetera::base_strategy::Xdg::new()?
        .state_dir()
        .unwrap_or_default()
        .join("glos/glos.log");
    std::fs::create_dir_all(path.parent().unwrap())?;
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    let level = match verbose {
        0 => tracing::Level::WARN,
        1 => tracing::Level::INFO,
        2 => tracing::Level::DEBUG,
        _ => tracing::Level::TRACE,
    };
    tracing_subscriber::fmt()
        .with_writer(std::sync::Mutex::new(file))
        .with_ansi(false)
        .with_max_level(level)
        .init();
    Ok(path)
}

fn run() -> Result<ExitCode> {
    let cli = Cli::parse();
    let log = init_log(cli.verbose).ok();
    let db = match cli.db {
        Some(p) => p,
        None => default_db()?,
    };
    if let Some(Command::Doctor) = cli.command {
        doctor::run(&mut std::io::stdout(), &db, log.as_deref())?;
        return Ok(ExitCode::SUCCESS);
    }
    let _conn = store::open(&db)?;

    let (config, config_error) = config::load();
    let source = match cli.command {
        Some(Command::Doctor) => unreachable!(),
        Some(Command::Diff { old, new }) => Source::diff(&old, &new)?,
        None => match Source::git(&std::env::current_dir()?, None) {
            Ok(s) => s,
            Err(e) if gix::discover(".").is_err() => {
                eprintln!("glos: not in a git repository ({e:#}).");
                eprintln!("To review two files or folders: glos diff <old> <new>");
                return Ok(ExitCode::from(2));
            }
            Err(e) => return Err(e),
        },
    };
    if source.changes.is_empty() {
        match &source.git {
            Some(g) => println!("Nothing to review: no changes since {}.", g.base),
            None => println!("Nothing to review: no differences."),
        }
        return Ok(ExitCode::SUCCESS);
    }
    if let Some(e) = config_error {
        // As Helix: say what's wrong, then carry on with the defaults.
        eprintln!("Bad config: {e}");
        eprintln!("Press <ENTER> to continue with the default config");
        let _ = std::io::stdin().read_line(&mut String::new());
    }
    ui::run(source, &config)?;
    Ok(ExitCode::SUCCESS)
}
