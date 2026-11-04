//! glos's maintenance tasks, run with `cargo xtask <task>`:
//!
//! - `fetch-helix [lang …]`: vendors what glos takes from the pinned Helix release into
//!   `crates/glos-helix` (rerun to move to a new release).
//! - `keymap [default.rs]`: writes `plans/KEYMAP.md`, the keymap checklist.

mod fetch_helix;
mod keymap;

use std::path::PathBuf;

use anyhow::{Result, bail};

/// The Helix release glos follows. Moving to a new one: change it here and in the Cargo.toml of
/// `crates/glos-helix`, then rerun both tasks.
pub const HELIX_TAG: &str = "25.07.1";

/// The workspace root.
pub fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("fetch-helix") => fetch_helix::run(&args[1..]),
        Some("keymap") => keymap::run(args.get(1).map(PathBuf::from)),
        _ => bail!(
            "usage: cargo xtask fetch-helix [lang …]\n       cargo xtask keymap [path/to/helix-term/src/keymap/default.rs]"
        ),
    }
}

/// The body of a URL.
pub fn fetch(url: &str) -> Result<Vec<u8>> {
    eprintln!("fetch {url}");
    Ok(ureq::get(url)
        .call()?
        .body_mut()
        .with_config()
        .limit(512 * 1024 * 1024)
        .read_to_vec()?)
}
