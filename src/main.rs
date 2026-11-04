mod embedded;
mod source;
mod store;
mod syntax;
mod theme;

use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;
use etcetera::BaseStrategy;

/// A terminal code reviewer for work done by coding agents.
#[derive(Parser)]
#[command(version)]
struct Cli {
    /// The database to use instead of `$XDG_DATA_HOME/glos/glos.db`.
    #[arg(long, env = "GLOS_DB", global = true)]
    db: Option<PathBuf>,
}

/// XDG folders on macOS too, as Helix does.
fn default_db() -> Result<PathBuf> {
    Ok(etcetera::base_strategy::Xdg::new()?.data_dir().join("glos/glos.db"))
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let db = match cli.db {
        Some(p) => p,
        None => default_db()?,
    };
    let _conn = store::open(&db)?;
    Ok(())
}
