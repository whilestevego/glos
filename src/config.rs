//! `~/.config/glos/config.toml`, laid out as Helix's: `theme`, and `[editor]`.

use std::path::PathBuf;

use anyhow::Result;
use serde::Deserialize;

#[derive(Deserialize, Debug, PartialEq)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct Config {
    pub theme: String,
    pub editor: Editor,
}

#[derive(Deserialize, Debug, PartialEq)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct Editor {
    /// Helix's `editor.mouse`: on unless turned off.
    pub mouse: bool,
    pub soft_wrap: SoftWrap,
}

/// Helix's `[editor.soft-wrap]`: off by default, as in Helix.
#[derive(Deserialize, Debug, PartialEq, Default)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct SoftWrap {
    pub enable: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            theme: "default".into(),
            editor: Editor::default(),
        }
    }
}

impl Default for Editor {
    fn default() -> Self {
        Editor {
            mouse: true,
            soft_wrap: SoftWrap::default(),
        }
    }
}

pub fn dir() -> Result<PathBuf> {
    use etcetera::BaseStrategy;
    Ok(etcetera::base_strategy::Xdg::new()?
        .config_dir()
        .join("glos"))
}

pub fn path() -> Result<PathBuf> {
    Ok(dir()?.join("config.toml"))
}

/// The config, or the defaults and what was wrong with the file. A missing file isn't an error.
pub fn load() -> (Config, Option<String>) {
    let Ok(path) = path() else {
        return (Config::default(), None);
    };
    match std::fs::read_to_string(&path) {
        Err(_) => (Config::default(), None),
        Ok(text) => match parse(&text) {
            Ok(c) => (c, None),
            Err(e) => (Config::default(), Some(format!("{}: {e}", path.display()))),
        },
    }
}

pub fn parse(text: &str) -> Result<Config, toml::de::Error> {
    toml::from_str(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_helix_layout() {
        assert_eq!(parse("").unwrap(), Config::default());
        let c = parse("theme = \"catppuccin_mocha\"\n[editor]\nmouse = false\n").unwrap();
        assert_eq!(c.theme, "catppuccin_mocha");
        assert!(!c.editor.mouse);
        assert!(
            parse("[editor.soft-wrap]\nenable = true\n")
                .unwrap()
                .editor
                .soft_wrap
                .enable
        );
        // Typos are errors, as in Helix, so they don't pass silently.
        assert!(parse("[editor]\nmose = false\n").is_err());
    }

    #[test]
    fn errors_name_the_problem() {
        let e = parse("theme = 3\n").unwrap_err().to_string();
        assert!(e.contains("theme"), "{e}");
        let e = parse("[editor.soft-wrap]\nenable = \"yes\"\n")
            .unwrap_err()
            .to_string();
        assert!(e.contains("enable"), "{e}");
        let e = parse("[keys.normal]\nx = \"y\"\n").unwrap_err().to_string();
        assert!(e.contains("keys"), "{e}");
        assert!(
            parse("theme = \"x\"\ntheme = \"y\"\n").is_err(),
            "duplicate keys"
        );
    }
}
