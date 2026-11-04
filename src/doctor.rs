//! `glos doctor`: checks the setup, like `hx --health`.

use std::{fmt::Write as _, io::Write, path::Path};

use crate::{
    clipboard, config,
    syntax::Loader,
    theme::{ColorSupport, Theme},
};

pub fn run(out: &mut impl Write, db: &Path, log: Option<&Path>) -> std::io::Result<()> {
    out.write_all(report(db, log).as_bytes())
}

fn report(db: &Path, log: Option<&Path>) -> String {
    let mut r = String::new();
    // Writing to a String can't fail.
    let mut line = |s: String| writeln!(r, "{s}").unwrap();
    line(format!(
        "glos {} (Helix 25.07.1)",
        env!("CARGO_PKG_VERSION")
    ));

    match crate::store::open(db) {
        Ok(conn) => {
            let v: u32 = conn
                .pragma_query_value(None, "user_version", |r| r.get(0))
                .unwrap_or(0);
            line(format!("Database: {} (schema v{v})", db.display()));
        }
        Err(e) => line(format!("Database: {} — error: {e:#}", db.display())),
    }

    let (cfg, error) = config::load();
    line(match (config::path(), error) {
        (_, Some(e)) => format!("Config: error, using defaults: {e}"),
        (Ok(p), None) if p.exists() => format!("Config: {}", p.display()),
        (Ok(p), None) => format!("Config: {} (not found, using defaults)", p.display()),
        (Err(e), None) => format!("Config: no config folder: {e}"),
    });
    let colors = ColorSupport::detect();
    let theme = match Theme::load(&cfg.theme, colors) {
        Ok(t) => {
            line(format!("Theme: {}", cfg.theme));
            t
        }
        Err(e) => {
            line(format!(
                "Theme: {} — error: {e:#}; using default",
                cfg.theme
            ));
            Theme::load("default", colors).expect("the default theme is embedded")
        }
    };
    line(format!(
        "Log: {}",
        log.map_or("unavailable".into(), |p| p.display().to_string())
    ));
    line(format!("Clipboard: {}", clipboard::provider()));
    let support = match colors {
        ColorSupport::TrueColor => "truecolor",
        ColorSupport::Indexed => "256 colours (COLORTERM isn't truecolor)",
        ColorSupport::None => "none (NO_COLOR is set)",
    };
    line(format!(
        "Terminal: TERM={}, colour: {support}",
        std::env::var("TERM").unwrap_or_default()
    ));
    line(format!(
        "Mouse: {}",
        if cfg.editor.mouse { "on" } else { "off" }
    ));

    match Loader::new(&theme) {
        Ok(loader) => {
            let langs = loader.health();
            let ok: Vec<_> = langs
                .iter()
                .filter(|(_, ok)| *ok)
                .map(|(n, _)| *n)
                .collect();
            let broken: Vec<_> = langs
                .iter()
                .filter(|(_, ok)| !ok)
                .map(|(n, _)| *n)
                .collect();
            line(format!("Languages ({}): {}", ok.len(), ok.join(", ")));
            if !broken.is_empty() {
                line(format!(
                    "Languages with a broken highlight query: {}",
                    broken.join(", ")
                ));
            }
        }
        Err(e) => line(format!("Languages: error: {e:#}")),
    }
    line(
        match crate::store::open(db).and_then(|c| {
            Ok(c.query_row("SELECT count(*) FROM sessions", [], |r| r.get::<_, i64>(0))?)
        }) {
            Ok(n) => format!(
                "MCP: {n} agent session{} attached; set up with `claude mcp add glos -- glos mcp`",
                if n == 1 { "" } else { "s" }
            ),
            Err(_) => "MCP: no database, so no sessions".into(),
        },
    );
    r
}

#[cfg(test)]
mod tests {
    #[test]
    fn reports_every_part() {
        let dir = tempfile::tempdir().unwrap();
        let mut out = vec![];
        super::run(&mut out, &dir.path().join("glos.db"), None).unwrap();
        let out = String::from_utf8(out).unwrap();
        for part in [
            "Database:",
            "(schema v4)",
            "Config:",
            "Theme:",
            "Log: unavailable",
            "Clipboard:",
            "Terminal:",
            "Languages (20):",
            "rust",
            "MCP: 0 agent sessions attached",
        ] {
            assert!(out.contains(part), "{part} missing from:\n{out}");
        }
        assert!(!out.contains("broken"), "{out}");
    }

    #[test]
    fn a_bad_database_is_reported_not_fatal() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("f");
        std::fs::write(&file, "x").unwrap();
        let out = super::report(&file.join("glos.db"), None);
        assert!(out.contains("Database:") && out.contains("error"), "{out}");
        assert!(
            out.contains("Languages (20)"),
            "the rest still runs:\n{out}"
        );
    }
}
