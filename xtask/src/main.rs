//! glos's maintenance tasks, run with `cargo xtask <task>`:
//!
//! - `fetch-helix [lang …]`: vendors what glos takes from the pinned Helix release into
//!   `crates/glos-helix` (rerun to move to a new release).
//! - `keymap [default.rs]`: writes `plans/KEYMAP.md`, the keymap checklist.
//! - `prose [base]`: runs Vale on the lines that this branch adds since it left `base` (`main`).
//! - `prose-test`: checks that each Vale rule fires on its fixture.

#![expect(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "xtask is a command-line tool, and the terminal is its interface"
)]

mod fetch_helix;
mod keymap;
mod prose;
mod vale;

use std::path::PathBuf;

use anyhow::{Result, bail};

/// The Helix release glos follows. Moving to a new one: change it here and in the Cargo.toml of
/// `crates/glos-helix`, then rerun both tasks.
pub const HELIX_TAG: &str = "25.07.1";

/// The workspace root.
#[must_use]
pub fn root() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/.."))
}

const USAGE: &str = "\
fetch-helix [lang …]   vendor the pinned Helix release into crates/glos-helix
keymap [default.rs]    write plans/KEYMAP.md
prose [base]           lint the prose on lines added since base (default main)
prose-test             check that each prose rule fires on its fixture";

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("fetch-helix") => fetch_helix::run(&args[1..]),
        Some("keymap") => keymap::run(args.get(1).map(PathBuf::from)),
        Some("prose") => prose::run(args.get(1).map_or("main", String::as_str)),
        Some("prose-test") => prose::self_test(),
        _ => bail!("usage: cargo xtask <task>\n\n{USAGE}"),
    }
}

/// The body of a URL.
///
/// # Errors
///
/// Fails when the request fails or the body cannot be read.
pub fn fetch(url: &str) -> Result<Vec<u8>> {
    eprintln!("fetch {url}");
    Ok(ureq::get(url)
        .call()?
        .body_mut()
        .with_config()
        .limit(512 * 1024 * 1024)
        .read_to_vec()?)
}
