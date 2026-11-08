//! `~/.config/glos/config.toml`, laid out as Helix's: `theme`, and `[editor]`.

use std::path::PathBuf;

use anyhow::Result;
use serde::Deserialize;

/// The theme when `config.toml` sets none, or names one that won't load. Helix's own `default`
/// is low in contrast, and collapsed thread markers are hard to read in it.
pub const DEFAULT_THEME: &str = "catppuccin_mocha";

#[derive(Deserialize, Debug, PartialEq)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct Config {
    pub theme: String,
    /// How glos opens itself when an agent asks for review or on `glos open`: the start of a
    /// shell command that runs the rest in a new pane, e.g. `tmux split-window -h`. glos adds
    /// its own command line.
    pub open_command: Option<String>,
    pub editor: Editor,
}

#[derive(Deserialize, Debug, PartialEq)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct Editor {
    /// Helix's `editor.mouse`: on unless turned off.
    pub mouse: bool,
    pub soft_wrap: SoftWrap,
    /// glos's: the file tree is open when glos starts (`space e` focuses it, `q` closes it).
    pub file_tree: bool,
    /// glos's: unchanged lines kept around each change; the rest are folded. `z+` and `z-`
    /// change it while glos runs.
    pub diff_context: usize,
    /// glos's: every changed file in one scroll, each under a header. `' v a` switches between
    /// this and one file at a time while glos runs.
    pub all_files: bool,
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
            theme: DEFAULT_THEME.into(),
            open_command: None,
            editor: Editor::default(),
        }
    }
}

impl Default for Editor {
    fn default() -> Self {
        Editor {
            mouse: true,
            soft_wrap: SoftWrap::default(),
            file_tree: true,
            diff_context: 3,
            all_files: true,
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
    fn open_command_is_top_level() {
        assert_eq!(Config::default().open_command, None);
        let c = parse("open-command = \"tmux split-window -h\"\n").unwrap();
        assert_eq!(c.open_command.as_deref(), Some("tmux split-window -h"));
    }

    #[test]
    fn the_file_tree_can_be_off() {
        assert!(Config::default().editor.file_tree);
        assert!(
            !parse("[editor]\nfile-tree = false\n")
                .unwrap()
                .editor
                .file_tree
        );
    }

    #[test]
    fn all_files_is_on_unless_turned_off() {
        assert!(Config::default().editor.all_files);
        assert!(
            !parse("[editor]\nall-files = false\n")
                .unwrap()
                .editor
                .all_files
        );
    }

    #[test]
    fn diff_context_defaults_to_three() {
        assert_eq!(Config::default().editor.diff_context, 3);
        let c = parse("[editor]\ndiff-context = 0\n").unwrap();
        assert_eq!(c.editor.diff_context, 0);
        assert!(parse("[editor]\ndiff-context = -1\n").is_err());
        assert!(parse("[editor]\ndiff-context = \"3\"\n").is_err());
    }

    #[test]
    fn the_default_theme_is_catppuccin_mocha_and_loads() {
        assert_eq!(Config::default().theme, "catppuccin_mocha");
        assert_eq!(parse("").unwrap().theme, DEFAULT_THEME);
        crate::theme::Theme::load(DEFAULT_THEME, crate::theme::ColorSupport::TrueColor).unwrap();
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
