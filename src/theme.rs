//! Helix themes: the embedded ones and the user's own in `~/.config/glos/themes/`, resolved as
//! helix-view does (`inherits`, `[palette]`, scope fallback).

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};

use anyhow::{Result, anyhow};
use ratatui::style::{Color, Modifier, Style};
use toml::{Table, Value};

use glos_helix::THEMES;

pub struct Theme {
    styles: HashMap<String, Style>,
    /// Every key, in the order syntax highlights are numbered (see `syntax`).
    scopes: Vec<String>,
}

/// How many colours the terminal shows, as Helix checks it.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ColorSupport {
    TrueColor,
    Indexed,
    None,
}

impl ColorSupport {
    pub fn detect() -> Self {
        if std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty()) {
            return ColorSupport::None;
        }
        match std::env::var("COLORTERM").as_deref() {
            Ok("truecolor" | "24bit") => ColorSupport::TrueColor,
            _ => ColorSupport::Indexed,
        }
    }
}

impl Theme {
    pub fn load(name: &str, colors: ColorSupport) -> Result<Theme> {
        Self::load_from(name, colors, user_dir().as_deref())
    }

    /// Loads a theme, looking in `user_dir` before the embedded ones.
    pub fn load_from(name: &str, colors: ColorSupport, user_dir: Option<&Path>) -> Result<Theme> {
        let mut table = resolve(name, user_dir, &mut HashSet::new())?;
        let palette = palette(table.remove("palette"));
        table.remove("inherits");
        let mut styles = HashMap::new();
        let mut scopes = vec![];
        for (key, value) in table {
            let style = adapt(parse_style(&value, &palette), colors);
            scopes.push(key.clone());
            styles.insert(key, style);
        }
        Ok(Theme { styles, scopes })
    }

    /// The style for `scope`, falling back to broader scopes (`ui.linenr.selected` →
    /// `ui.linenr` → `ui`), as Helix's `Theme::get`.
    pub fn get(&self, scope: &str) -> Style {
        self.try_get(scope).unwrap_or_default()
    }

    pub fn try_get(&self, scope: &str) -> Option<Style> {
        std::iter::successors(Some(scope), |s| Some(s.rsplit_once('.')?.0))
            .find_map(|s| self.styles.get(s).copied())
    }

    pub fn try_get_exact(&self, scope: &str) -> Option<Style> {
        self.styles.get(scope).copied()
    }

    pub fn scopes(&self) -> &[String] {
        &self.scopes
    }

    pub fn highlight(&self, index: usize) -> Style {
        self.styles[&self.scopes[index]]
    }
}

fn user_dir() -> Option<PathBuf> {
    use etcetera::BaseStrategy;
    Some(
        etcetera::base_strategy::Xdg::new()
            .ok()?
            .config_dir()
            .join("glos/themes"),
    )
}

/// Finds and merges a theme as helix-view's `load_theme` does: the user's folder first, then
/// the embedded themes. A theme already visited is skipped, so a user theme can inherit the
/// built-in theme of the same name; a real cycle is an error. `default` is always the built-in.
fn resolve(
    name: &str,
    user_dir: Option<&Path>,
    visited: &mut HashSet<(bool, String)>,
) -> Result<Table> {
    let embedded = |n: &str| {
        THEMES
            .iter()
            .find(|(t, _)| *t == n)
            .map(|(_, t)| t.to_string())
    };
    let mut cycle = false;
    let text = [true, false]
        .into_iter()
        .find_map(|user| {
            let text = if user {
                std::fs::read_to_string(user_dir?.join(format!("{name}.toml"))).ok()?
            } else {
                embedded(name)?
            };
            if !visited.insert((user, name.to_string())) {
                cycle = true;
                return None;
            }
            Some(text)
        })
        .ok_or_else(|| {
            if cycle {
                anyhow!("theme {name}: cycle found in inheriting")
            } else {
                anyhow!("no theme named {name}")
            }
        })?;
    let theme: Table = text.parse().map_err(|e| anyhow!("theme {name}: {e}"))?;
    match theme.get("inherits") {
        None => Ok(theme),
        Some(Value::String(parent)) => {
            let parent = if parent == "default" {
                embedded("default")
                    .unwrap_or_default()
                    .parse()
                    .map_err(|e| anyhow!("default theme: {e}"))?
            } else {
                resolve(parent, user_dir, visited)?
            };
            Ok(merge_themes(parent, theme))
        }
        Some(other) => Err(anyhow!(
            "theme {name}: expected 'inherits' to be a string: {other}"
        )),
    }
}

fn merge_themes(parent: Table, child: Table) -> Table {
    // The palette merges one level deeper than the rest of the theme (helix-view theme.rs).
    let empty = || Value::Table(Table::new());
    let palette = merge(
        parent.get("palette").cloned().unwrap_or_else(empty),
        child.get("palette").cloned().unwrap_or_else(empty),
        2,
    );
    let mut theme = merge(Value::Table(parent), Value::Table(child), 1);
    theme
        .as_table_mut()
        .unwrap()
        .insert("palette".into(), palette);
    match theme {
        Value::Table(t) => t,
        _ => unreachable!(),
    }
}

/// helix-loader's `merge_toml_values`, for tables (themes have no arrays to merge).
fn merge(left: Value, right: Value, depth: usize) -> Value {
    match (left, right) {
        (Value::Table(mut l), Value::Table(r)) if depth > 0 => {
            for (k, rv) in r {
                let v = match l.remove(&k) {
                    Some(lv) => merge(lv, rv, depth - 1),
                    None => rv,
                };
                l.insert(k, v);
            }
            Value::Table(l)
        }
        (_, r) => r,
    }
}

/// Helix's default palette (the 16 terminal colours), overridden by the theme's `[palette]`.
fn palette(value: Option<Value>) -> HashMap<String, Color> {
    let mut p: HashMap<String, Color> = [
        ("default", Color::Reset),
        ("black", Color::Black),
        ("red", Color::Red),
        ("green", Color::Green),
        ("yellow", Color::Yellow),
        ("blue", Color::Blue),
        ("magenta", Color::Magenta),
        ("cyan", Color::Cyan),
        // Helix's gray is terminal colour 8 and light-gray is 7 (helix-view graphics.rs).
        ("gray", Color::DarkGray),
        ("light-red", Color::LightRed),
        ("light-green", Color::LightGreen),
        ("light-yellow", Color::LightYellow),
        ("light-blue", Color::LightBlue),
        ("light-magenta", Color::LightMagenta),
        ("light-cyan", Color::LightCyan),
        ("light-gray", Color::Gray),
        ("white", Color::White),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v))
    .collect();
    if let Some(Value::Table(t)) = value {
        for (k, v) in t {
            if let Some(c) = v.as_str().and_then(hex) {
                p.insert(k, c);
            }
        }
    }
    p
}

fn hex(s: &str) -> Option<Color> {
    let h = s.strip_prefix('#')?;
    if h.len() != 6 {
        return None;
    }
    let n = |i| u8::from_str_radix(&h[i..i + 2], 16).ok();
    Some(Color::Rgb(n(0)?, n(2)?, n(4)?))
}

fn color(v: &Value, palette: &HashMap<String, Color>) -> Option<Color> {
    match v {
        Value::String(s) => palette.get(s).copied().or_else(|| hex(s)),
        Value::Integer(i) => u8::try_from(*i).ok().map(Color::Indexed),
        _ => None,
    }
}

fn parse_style(v: &Value, palette: &HashMap<String, Color>) -> Style {
    let mut style = Style::default();
    let Value::Table(t) = v else {
        // `"keyword" = "red"` is shorthand for a foreground.
        if let Some(c) = color(v, palette) {
            style = style.fg(c);
        }
        return style;
    };
    if let Some(c) = t.get("fg").and_then(|v| color(v, palette)) {
        style = style.fg(c);
    }
    if let Some(c) = t.get("bg").and_then(|v| color(v, palette)) {
        style = style.bg(c);
    }
    if let Some(Value::Table(u)) = t.get("underline") {
        if let Some(c) = u.get("color").and_then(|v| color(v, palette)) {
            style = style.underline_color(c);
        }
        // ponytail: every underline style (curl, dotted…) draws as a plain underline.
        if u.get("style").is_some() {
            style = style.add_modifier(Modifier::UNDERLINED);
        }
    }
    for m in t
        .get("modifiers")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let m = match m.as_str() {
            Some("bold") => Modifier::BOLD,
            Some("dim") => Modifier::DIM,
            Some("italic") => Modifier::ITALIC,
            Some("underlined") => Modifier::UNDERLINED,
            Some("slow_blink") => Modifier::SLOW_BLINK,
            Some("rapid_blink") => Modifier::RAPID_BLINK,
            Some("reversed") => Modifier::REVERSED,
            Some("hidden") => Modifier::HIDDEN,
            Some("crossed_out") => Modifier::CROSSED_OUT,
            _ => continue,
        };
        style = style.add_modifier(m);
    }
    style
}

/// Maps 24-bit colours to the nearest of the 256, or drops colour for `NO_COLOR`.
fn adapt(style: Style, colors: ColorSupport) -> Style {
    let map = |c: Option<Color>| match (c, colors) {
        (_, ColorSupport::None) => None,
        (Some(Color::Rgb(r, g, b)), ColorSupport::Indexed) => {
            Some(Color::Indexed(nearest_256(r, g, b)))
        }
        (c, _) => c,
    };
    Style {
        fg: map(style.fg),
        bg: map(style.bg),
        underline_color: map(style.underline_color),
        ..style
    }
}

/// The nearest xterm-256 colour: the 6×6×6 cube or the 24-step grey ramp.
pub fn nearest_256(r: u8, g: u8, b: u8) -> u8 {
    const STEPS: [u8; 6] = [0, 95, 135, 175, 215, 255];
    let level = |v: u8| {
        STEPS
            .iter()
            .enumerate()
            .min_by_key(|(_, s)| (**s as i32 - v as i32).abs())
            .unwrap()
            .0
    };
    let (ri, gi, bi) = (level(r), level(g), level(b));
    let cube = (STEPS[ri], STEPS[gi], STEPS[bi]);
    let avg = (r as u32 + g as u32 + b as u32) / 3;
    let gi_ = ((avg.saturating_sub(8)) / 10).min(23) as u8;
    let grey = 8 + 10 * gi_;
    let dist = |(x, y, z): (u8, u8, u8)| {
        let d = |a: u8, b: u8| (a as i32 - b as i32).pow(2);
        d(x, r) + d(y, g) + d(z, b)
    };
    if dist((grey, grey, grey)) < dist(cube) {
        232 + gi_
    } else {
        16 + 36 * ri as u8 + 6 * gi as u8 + bi as u8
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inherits_and_palette() {
        // catppuccin_frappe inherits catppuccin_mocha but overrides its palette.
        let t = Theme::load("catppuccin_frappe", ColorSupport::TrueColor).unwrap();
        assert_eq!(t.get("keyword").fg, Some(Color::Rgb(0xca, 0x9e, 0xe6)));
        let m = Theme::load("catppuccin_mocha", ColorSupport::TrueColor).unwrap();
        assert_eq!(m.get("keyword").fg, Some(Color::Rgb(0xcb, 0xa6, 0xf7)));
    }

    #[test]
    fn scope_fallback_and_colour_support() {
        let t = Theme::load("default", ColorSupport::TrueColor).unwrap();
        assert_eq!(
            t.try_get("ui.linenr.selected.nonexistent"),
            t.try_get("ui.linenr.selected")
        );
        assert!(t.try_get_exact("keyword.control.nonexistent").is_none());
        let none = Theme::load("default", ColorSupport::None).unwrap();
        assert!(
            none.scopes()
                .iter()
                .all(|s| none.get(s).fg.is_none() && none.get(s).bg.is_none())
        );
        let idx = Theme::load("catppuccin_mocha", ColorSupport::Indexed).unwrap();
        assert!(matches!(idx.get("keyword").fg, Some(Color::Indexed(_))));
    }

    #[test]
    fn every_embedded_theme_loads() {
        for (name, _) in THEMES {
            Theme::load(name, ColorSupport::TrueColor).unwrap_or_else(|e| panic!("{name}: {e}"));
        }
    }

    #[test]
    fn nearest_colours() {
        assert_eq!(nearest_256(0, 0, 0), 16);
        assert_eq!(nearest_256(255, 255, 255), 231);
        assert_eq!(nearest_256(255, 0, 0), 196);
        assert_eq!(nearest_256(128, 128, 128), 244);
    }

    fn user(themes: &[(&str, &str)]) -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        for (n, t) in themes {
            std::fs::write(d.path().join(format!("{n}.toml")), t).unwrap();
        }
        d
    }

    #[test]
    fn user_themes_win_and_can_extend_a_built_in_of_the_same_name() {
        let d = user(&[
            ("mine", "keyword = \"#010203\"\n"),
            ("default", "inherits = \"default\"\nkeyword = \"#0a0b0c\"\n"),
        ]);
        let mine = Theme::load_from("mine", ColorSupport::TrueColor, Some(d.path())).unwrap();
        assert_eq!(mine.get("keyword").fg, Some(Color::Rgb(1, 2, 3)));
        // The user's `default` inherits the built-in one, then overrides a key.
        let def = Theme::load_from("default", ColorSupport::TrueColor, Some(d.path())).unwrap();
        let builtin = Theme::load_from("default", ColorSupport::TrueColor, None).unwrap();
        assert_eq!(def.get("keyword").fg, Some(Color::Rgb(10, 11, 12)));
        assert_eq!(def.get("ui.linenr"), builtin.get("ui.linenr"));
    }

    #[test]
    fn theme_errors() {
        let d = user(&[
            ("a", "inherits = \"b\"\n"),
            ("b", "inherits = \"a\"\n"),
            ("bad", "this is not toml ="),
            ("num", "inherits = 3\n"),
        ]);
        let err = |n| {
            Theme::load_from(n, ColorSupport::TrueColor, Some(d.path()))
                .err()
                .unwrap()
                .to_string()
        };
        assert!(err("a").contains("cycle"), "{}", err("a"));
        assert!(err("bad").contains("theme bad"), "{}", err("bad"));
        assert!(err("num").contains("expected 'inherits' to be a string"));
        assert!(err("missing").contains("no theme named missing"));
    }

    #[test]
    fn style_parsing() {
        let d = user(&[(
            "t",
            r##"
"a" = "red"
"b" = { fg = 33, bg = "#102030", modifiers = ["bold", "italic", "crossed_out", "unknown"] }
"c" = { fg = "accent", underline = { color = "red", style = "curl" } }
"d" = "light-gray"
[palette]
accent = "#abcdef"
"##,
        )]);
        let t = Theme::load_from("t", ColorSupport::TrueColor, Some(d.path())).unwrap();
        assert_eq!(t.get("a").fg, Some(Color::Red));
        let b = t.get("b");
        assert_eq!(
            (b.fg, b.bg),
            (Some(Color::Indexed(33)), Some(Color::Rgb(0x10, 0x20, 0x30)))
        );
        assert!(
            b.add_modifier
                .contains(Modifier::BOLD | Modifier::ITALIC | Modifier::CROSSED_OUT)
        );
        let c = t.get("c");
        assert_eq!(c.fg, Some(Color::Rgb(0xab, 0xcd, 0xef)));
        assert_eq!(c.underline_color, Some(Color::Red));
        assert!(c.add_modifier.contains(Modifier::UNDERLINED));
        assert_eq!(
            t.get("d").fg,
            Some(Color::Gray),
            "Helix's light-gray is terminal colour 7"
        );
        assert_eq!(t.get("nothing"), Style::default());
    }
}
