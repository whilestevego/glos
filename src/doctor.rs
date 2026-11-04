//! `glos doctor`: checks the setup, like `hx --health`.

use std::path::Path;

use crate::{
    clipboard, config,
    syntax::Loader,
    theme::{ColorSupport, Theme},
};

pub fn run(db: &Path, log: Option<&Path>) {
    println!("glos {} (Helix 25.07.1)", env!("CARGO_PKG_VERSION"));

    match crate::store::open(db) {
        Ok(conn) => {
            let v: u32 = conn
                .pragma_query_value(None, "user_version", |r| r.get(0))
                .unwrap_or(0);
            println!("Database: {} (schema v{v})", db.display());
        }
        Err(e) => println!("Database: {} — error: {e:#}", db.display()),
    }

    let (cfg, error) = config::load();
    match (config::path(), error) {
        (Ok(p), None) if p.exists() => println!("Config: {}", p.display()),
        (Ok(p), None) => println!("Config: {} (not found, using defaults)", p.display()),
        (_, Some(e)) => println!("Config: error, using defaults: {e}"),
        (Err(e), None) => println!("Config: no config folder: {e}"),
    }
    let colors = ColorSupport::detect();
    let theme = match Theme::load(&cfg.theme, colors) {
        Ok(t) => {
            println!("Theme: {}", cfg.theme);
            t
        }
        Err(e) => {
            println!("Theme: {} — error: {e:#}; using default", cfg.theme);
            Theme::load("default", colors).expect("the default theme is embedded")
        }
    };
    println!(
        "Log: {}",
        log.map_or("unavailable".into(), |p| p.display().to_string())
    );
    println!("Clipboard: {}", clipboard::provider());
    let term = std::env::var("TERM").unwrap_or_default();
    let support = match colors {
        ColorSupport::TrueColor => "truecolor",
        ColorSupport::Indexed => "256 colours (COLORTERM isn't truecolor)",
        ColorSupport::None => "none (NO_COLOR is set)",
    };
    println!("Terminal: TERM={term}, colour: {support}");
    println!("Mouse: {}", if cfg.editor.mouse { "on" } else { "off" });

    match Loader::new(&theme) {
        Ok(loader) => {
            let langs = loader.health();
            let broken: Vec<_> = langs
                .iter()
                .filter(|(_, ok)| !ok)
                .map(|(n, _)| *n)
                .collect();
            let ok: Vec<_> = langs
                .iter()
                .filter(|(_, ok)| *ok)
                .map(|(n, _)| *n)
                .collect();
            println!("Languages ({}): {}", ok.len(), ok.join(", "));
            if !broken.is_empty() {
                println!(
                    "Languages with a broken highlight query: {}",
                    broken.join(", ")
                );
            }
        }
        Err(e) => println!("Languages: error: {e:#}"),
    }
    println!("MCP: `glos mcp` comes in M2");
}
