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
        let mut raw = HashMap::new();
        let mut scopes = vec![];
        for (key, value) in table {
            scopes.push(key.clone());
            raw.insert(key, parse_style(&value, &palette));
        }
        // The diff tints are worked out from the theme's own RGB colours, before they're mapped
        // to the 256 colours. A theme that sets one of the keys keeps its own.
        let background = raw.get("ui.background").and_then(|s| rgb(s.bg?));
        for (key, toward, amount) in TINTS {
            if raw.contains_key(*key) {
                continue;
            }
            let fg = raw.get(*toward).and_then(|s| rgb(s.fg?));
            // Without both colours there's no tint. The key is still set, so that it doesn't
            // fall back to `diff.plus` and colour the text.
            let bg = match (background, fg, colors) {
                (Some(b), Some(f), ColorSupport::Indexed) => Some(tint_256(b, f, *amount)),
                (Some(b), Some(f), _) => Some(rgb_color(mix(b, f, *amount))),
                _ => None,
            };
            raw.insert(
                key.to_string(),
                Style {
                    bg,
                    ..Style::default()
                },
            );
        }
        let styles = raw
            .into_iter()
            .map(|(k, s)| (k, adapt(s, colors)))
            .collect();
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

/// The diff tints: each key, the style whose colour it leans toward, and how far from the
/// background it goes. The line tints are subtle, and the changed words stronger.
const TINTS: &[(&str, &str, f32)] = &[
    ("diff.plus.line", "diff.plus", 0.14),
    ("diff.minus.line", "diff.minus", 0.14),
    ("diff.plus.word", "diff.plus", 0.35),
    ("diff.minus.word", "diff.minus", 0.35),
];

type Rgb = (u8, u8, u8);

fn rgb(c: Color) -> Option<Rgb> {
    match c {
        Color::Rgb(r, g, b) => Some((r, g, b)),
        _ => None,
    }
}

fn rgb_color((r, g, b): Rgb) -> Color {
    Color::Rgb(r, g, b)
}

/// `from` moved `amount` of the way toward `to`, in OKLab. OKLab is perceptual, so the same
/// amount looks about as strong on a dark theme as on a light one, and mid-tones don't go muddy
/// as they do when RGB is mixed directly.
pub fn mix(from: Rgb, to: Rgb, amount: f32) -> Rgb {
    let (a, b) = (oklab(from), oklab(to));
    let t = amount.clamp(0.0, 1.0);
    from_oklab([0, 1, 2].map(|i| a[i] + (b[i] - a[i]) * t))
}

/// A tint for 256 colours: the mix, mapped to the nearest of the 256. A small tint can round to
/// the background's own colour, so the mix goes further toward `to` until it doesn't.
fn tint_256(from: Rgb, to: Rgb, amount: f32) -> Color {
    let bg = nearest_256(from.0, from.1, from.2);
    let mut t = amount;
    loop {
        let (r, g, b) = mix(from, to, t);
        let c = nearest_256(r, g, b);
        if c != bg || t >= 1.0 {
            return Color::Indexed(c);
        }
        t = (t + 0.05).min(1.0);
    }
}

fn oklab((r, g, b): Rgb) -> [f32; 3] {
    let lin = |c: u8| {
        let c = c as f32 / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    let (r, g, b) = (lin(r), lin(g), lin(b));
    let l = (0.412_221_47 * r + 0.536_332_55 * g + 0.051_445_99 * b).cbrt();
    let m = (0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b).cbrt();
    let s = (0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b).cbrt();
    [
        0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s,
        1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s,
        0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s,
    ]
}

fn from_oklab([l, a, b]: [f32; 3]) -> Rgb {
    let l_ = (l + 0.396_337_78 * a + 0.215_803_76 * b).powi(3);
    let m_ = (l - 0.105_561_346 * a - 0.063_854_17 * b).powi(3);
    let s_ = (l - 0.089_484_18 * a - 1.291_485_5 * b).powi(3);
    let r = 4.076_741_7 * l_ - 3.307_711_6 * m_ + 0.230_969_94 * s_;
    let g = -1.268_438 * l_ + 2.609_757_4 * m_ - 0.341_319_38 * s_;
    let b = -0.004_196_086_3 * l_ - 0.703_418_6 * m_ + 1.707_614_7 * s_;
    let gamma = |c: f32| {
        let c = if c <= 0.003_130_8 {
            12.92 * c
        } else {
            1.055 * c.powf(1.0 / 2.4) - 0.055
        };
        (c.clamp(0.0, 1.0) * 255.0).round() as u8
    };
    (gamma(r), gamma(g), gamma(b))
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
    fn oklab_round_trips_and_mixes_between_its_ends() {
        for c in [
            (0, 0, 0),
            (255, 255, 255),
            (0x35, 0xbf, 0x86),
            (0x1e, 0x1e, 0x2e),
            (12, 200, 99),
        ] {
            let back = from_oklab(oklab(c));
            let close = |a: u8, b: u8| a.abs_diff(b) <= 1;
            assert!(
                close(back.0, c.0) && close(back.1, c.1) && close(back.2, c.2),
                "{c:?} → {back:?}"
            );
        }
        let (bg, green) = ((0x1e, 0x1e, 0x2e), (0xa6, 0xe3, 0xa1));
        assert_eq!(mix(bg, green, 0.0), bg);
        assert_eq!(mix(bg, green, 1.0), green);
        // Black to white at half way is OKLab's middle grey, not RGB's 128.
        let grey = mix((0, 0, 0), (255, 255, 255), 0.5);
        assert!(
            grey.0 == grey.1 && grey.1 == grey.2 && (95..=105).contains(&grey.0),
            "{grey:?}"
        );
        // A mix stays between its ends in every channel.
        let m = mix(bg, green, 0.18);
        for (x, (a, b)) in [
            (m.0, (bg.0, green.0)),
            (m.1, (bg.1, green.1)),
            (m.2, (bg.2, green.2)),
        ] {
            assert!(a.min(b) <= x && x <= a.max(b), "{m:?}");
        }
    }

    #[test]
    fn diff_tints_lean_from_the_background_toward_the_diff_colours() {
        let t = Theme::load("catppuccin_mocha", ColorSupport::TrueColor).unwrap();
        let base = (0x1e, 0x1e, 0x2e);
        let (green, red) = ((0xa6, 0xe3, 0xa1), (0xf3, 0x8b, 0xa8));
        let bg = |k| t.get(k).bg;
        assert_eq!(
            bg("diff.plus.line"),
            Some(rgb_color(mix(base, green, 0.14)))
        );
        assert_eq!(bg("diff.minus.line"), Some(rgb_color(mix(base, red, 0.14))));
        assert_eq!(
            bg("diff.plus.word"),
            Some(rgb_color(mix(base, green, 0.35)))
        );
        assert_eq!(bg("diff.minus.word"), Some(rgb_color(mix(base, red, 0.35))));
        // The tints only set a background, so syntax colours show through.
        assert_eq!(t.get("diff.plus.line").fg, None);
        // 256 colours: never the background's own colour.
        let idx = Theme::load("catppuccin_mocha", ColorSupport::Indexed).unwrap();
        for k in [
            "diff.plus.line",
            "diff.minus.line",
            "diff.plus.word",
            "diff.minus.word",
        ] {
            let c = idx.get(k).bg;
            assert!(matches!(c, Some(Color::Indexed(_))), "{k}");
            assert_ne!(c, idx.get("ui.background").bg, "{k}");
        }
        // NO_COLOR: no tints.
        let none = Theme::load("catppuccin_mocha", ColorSupport::None).unwrap();
        assert_eq!(none.get("diff.plus.line").bg, None);
    }

    #[test]
    fn a_theme_can_set_its_own_tints_and_one_without_a_background_gets_none() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("mine.toml"),
            "inherits = \"catppuccin_mocha\"\n\"diff.plus.line\" = { bg = \"#102030\" }\n",
        )
        .unwrap();
        let t = Theme::load_from("mine", ColorSupport::TrueColor, Some(dir.path())).unwrap();
        assert_eq!(
            t.get("diff.plus.line").bg,
            Some(Color::Rgb(0x10, 0x20, 0x30))
        );
        assert!(
            t.get("diff.minus.line").bg.is_some(),
            "the others are still worked out"
        );
        // No RGB background: no tint, and no fallback to `diff.plus`'s colour either.
        std::fs::write(
            dir.path().join("bare.toml"),
            "\"diff.plus\" = \"#00ff00\"\n",
        )
        .unwrap();
        let t = Theme::load_from("bare", ColorSupport::TrueColor, Some(dir.path())).unwrap();
        assert_eq!(t.get("diff.plus.line"), Style::default());
    }

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
