mod diff;
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

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Review two files or two folders, like `git diff --no-index`.
    Diff { old: PathBuf, new: PathBuf },
}

/// XDG folders on macOS too, as Helix does.
fn default_db() -> Result<PathBuf> {
    Ok(etcetera::base_strategy::Xdg::new()?.data_dir().join("glos/glos.db"))
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(e) => {
            eprintln!("glos: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<ExitCode> {
    let cli = Cli::parse();
    let db = match cli.db {
        Some(p) => p,
        None => default_db()?,
    };
    let _conn = store::open(&db)?;

    let source = match cli.command {
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
    ui::run(source, "default")?;
    Ok(ExitCode::SUCCESS)
}
