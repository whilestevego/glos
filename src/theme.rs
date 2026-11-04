//! Helix themes: the embedded ones and the user's own in `~/.config/glos/themes/`, resolved as
//! helix-view does (`inherits`, `[palette]`, scope fallback).

use std::{collections::HashMap, path::PathBuf};

use anyhow::{Result, anyhow};
use ratatui::style::{Color, Modifier, Style};
use toml::{Table, Value};

use crate::embedded::THEMES;

pub struct Theme {
    pub name: String,
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
        let mut table = resolve(name, 0)?;
        let palette = palette(table.remove("palette"));
        table.remove("inherits");
        let mut styles = HashMap::new();
        let mut scopes = vec![];
        for (key, value) in table {
            let style = adapt(parse_style(&value, &palette), colors);
            scopes.push(key.clone());
            styles.insert(key, style);
        }
        Ok(Theme { name: name.into(), styles, scopes })
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

fn user_theme(name: &str) -> Option<String> {
    let dir: PathBuf = etcetera::base_strategy::Xdg::new().ok().map(|x| {
        use etcetera::BaseStrategy;
        x.config_dir().join("glos/themes")
    })?;
    std::fs::read_to_string(dir.join(format!("{name}.toml"))).ok()
}

/// The user's theme of that name wins over the embedded one, as in Helix.
fn resolve(name: &str, depth: usize) -> Result<Table> {
    anyhow::ensure!(depth < 10, "theme {name}: `inherits` goes round in a circle");
    let text = match user_theme(name) {
        Some(t) => t,
        None => THEMES
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, t)| t.to_string())
            .ok_or_else(|| anyhow!("no theme named {name}"))?,
    };
    let theme: Table = text.parse().map_err(|e| anyhow!("theme {name}: {e}"))?;
    match theme.get("inherits").and_then(Value::as_str) {
        Some(parent) => Ok(merge_themes(resolve(parent, depth + 1)?, theme)),
        None => Ok(theme),
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
    theme.as_table_mut().unwrap().insert("palette".into(), palette);
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
    for m in t.get("modifiers").and_then(Value::as_array).into_iter().flatten() {
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
        (Some(Color::Rgb(r, g, b)), ColorSupport::Indexed) => Some(Color::Indexed(nearest_256(r, g, b))),
        (c, _) => c,
    };
    Style { fg: map(style.fg), bg: map(style.bg), underline_color: map(style.underline_color), ..style }
}

/// The nearest xterm-256 colour: the 6×6×6 cube or the 24-step grey ramp.
pub fn nearest_256(r: u8, g: u8, b: u8) -> u8 {
    const STEPS: [u8; 6] = [0, 95, 135, 175, 215, 255];
    let level = |v: u8| STEPS.iter().enumerate().min_by_key(|(_, s)| (**s as i32 - v as i32).abs()).unwrap().0;
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
        assert_eq!(t.try_get("ui.linenr.selected.nonexistent"), t.try_get("ui.linenr.selected"));
        assert!(t.try_get_exact("keyword.control.nonexistent").is_none());
        let none = Theme::load("default", ColorSupport::None).unwrap();
        assert!(none.scopes().iter().all(|s| none.get(s).fg.is_none() && none.get(s).bg.is_none()));
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
}
