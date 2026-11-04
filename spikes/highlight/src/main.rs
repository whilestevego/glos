//! Spike: highlight a file with grammars compiled into the binary, Helix's queries and themes.
//! Usage: highlight-spike <file> [theme]
use std::{collections::HashMap, time::Duration};

use ropey::Rope;
use toml::{Table, Value};
use tree_house::{
    highlighter::{Highlight, HighlightEvent, Highlighter},
    tree_sitter::Grammar,
    InjectionLanguageMarker, Language, LanguageConfig, LanguageLoader, Syntax,
};

include!(concat!(env!("OUT_DIR"), "/embedded.rs"));

struct Lang {
    name: String,
    file_types: Vec<String>,
    config: LanguageConfig,
}

struct Loader {
    langs: Vec<Lang>,
}

impl LanguageLoader for Loader {
    fn language_for_marker(&self, marker: InjectionLanguageMarker) -> Option<Language> {
        // ponytail: exact names only; Helix also matches injection-regex, filenames and shebangs.
        let name: String = match marker {
            InjectionLanguageMarker::Name(n) => n.into(),
            InjectionLanguageMarker::Match(text) => text.into(),
            _ => return None,
        };
        self.langs.iter().position(|l| l.name == name).map(|i| Language::new(i as u32))
    }
    fn get_config(&self, lang: Language) -> Option<&LanguageConfig> {
        Some(&self.langs[lang.idx()].config)
    }
}

fn query(lang: &str, file: &str) -> String {
    tree_house::read_query(lang, |l| {
        QUERIES.iter().find(|(ql, qf, _)| *ql == l && *qf == file).map(|q| q.2.to_string()).unwrap_or_default()
    })
}

fn loader(scopes: &[String]) -> Loader {
    let config: Table = LANGUAGES_TOML.parse().unwrap();
    let mut langs = Vec::new();
    for l in config["language"].as_array().unwrap() {
        let name = l["name"].as_str().unwrap();
        let grammar_name = l.get("grammar").and_then(Value::as_str).unwrap_or(name);
        let Some((_, f)) = GRAMMARS.iter().find(|(g, _)| *g == grammar_name) else { continue };
        let f = unsafe { tree_sitter_language::LanguageFn::from_raw(std::mem::transmute(*f)) };
        let grammar = Grammar::try_from(f).unwrap();
        let t = std::time::Instant::now();
        let config = LanguageConfig::new(
            grammar,
            &query(name, "highlights.scm"),
            &query(name, "injections.scm"),
            &query(name, "locals.scm"),
        )
        .unwrap_or_else(|e| panic!("{name}: {e}"));
        if std::env::var_os("TIMES").is_some() {
            eprintln!("compile {name}: {:?}", t.elapsed());
        }
        configure(&config, scopes);
        let file_types = l["file-types"].as_array().unwrap().iter().filter_map(|v| v.as_str().map(String::from)).collect();
        langs.push(Lang { name: name.into(), file_types, config });
    }
    Loader { langs }
}

/// Helix's scope matching: a capture takes the theme key with the longest matching prefix
/// (`keyword.control.import` → `keyword.control` → `keyword`). Copied from helix-core syntax.rs.
fn configure(config: &LanguageConfig, scopes: &[String]) {
    config.configure(|capture| {
        let parts: Vec<_> = capture.split('.').collect();
        let mut best = None;
        let mut best_len = 0;
        for (i, scope) in scopes.iter().enumerate() {
            let s: Vec<_> = scope.split('.').collect();
            if s.len() <= parts.len() && s.iter().zip(&parts).all(|(a, b)| a == b) && s.len() > best_len {
                best = Some(i);
                best_len = s.len();
            }
        }
        best.map(|i| Highlight::new(i as u32))
    });
}

// ---- Themes, as helix-view theme.rs does it ----

fn theme_toml(name: &str) -> Table {
    let text = THEMES.iter().find(|(n, _)| *n == name).unwrap_or_else(|| panic!("no theme {name}")).1;
    let theme: Table = text.parse().unwrap();
    match theme.get("inherits").and_then(Value::as_str) {
        Some(parent) => merge_themes(theme_toml(parent), theme),
        None => theme,
    }
}

fn merge_themes(parent: Table, child: Table) -> Table {
    // The palette merges one level deeper than the rest of the theme.
    let palette = merge(
        parent.get("palette").cloned().unwrap_or(Value::Table(Table::new())),
        child.get("palette").cloned().unwrap_or(Value::Table(Table::new())),
        2,
    );
    let mut theme = merge(Value::Table(parent), Value::Table(child), 1);
    theme.as_table_mut().unwrap().insert("palette".into(), palette);
    theme.try_into().unwrap()
}

/// helix-loader's merge_toml_values (tables only; themes have no arrays to merge).
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

#[derive(Clone, Copy, Default)]
struct Style {
    fg: Option<(u8, u8, u8)>,
    bold: bool,
    italic: bool,
    underline: bool,
}

impl Style {
    fn patch(mut self, o: Style) -> Style {
        self.fg = o.fg.or(self.fg);
        self.bold |= o.bold;
        self.italic |= o.italic;
        self.underline |= o.underline;
        self
    }
    fn ansi(self) -> String {
        let mut s = "\x1b[0".to_string();
        if let Some((r, g, b)) = self.fg {
            s += &format!(";38;2;{r};{g};{b}");
        }
        if self.bold { s += ";1" }
        if self.italic { s += ";3" }
        if self.underline { s += ";4" }
        s + "m"
    }
}

/// (scopes, styles) in the same order; highlight index i maps to styles[i].
fn theme(name: &str) -> (Vec<String>, Vec<Style>) {
    let mut t = theme_toml(name);
    let palette = t.remove("palette").and_then(|p| p.try_into::<HashMap<String, String>>().ok()).unwrap_or_default();
    t.remove("inherits");
    // ponytail: hex colours only; named ANSI colours and 256-colour indexes are skipped.
    let color = |v: &Value| {
        let s = v.as_str()?;
        let hex = palette.get(s).map(String::as_str).unwrap_or(s);
        let h = |i| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok();
        hex.starts_with('#').then(|| Some((h(1)?, h(3)?, h(5)?))).flatten()
    };
    let (mut scopes, mut styles) = (Vec::new(), Vec::new());
    for (k, v) in t {
        let mut st = Style::default();
        match &v {
            Value::Table(tb) => {
                st.fg = tb.get("fg").and_then(color);
                for m in tb.get("modifiers").and_then(Value::as_array).into_iter().flatten() {
                    match m.as_str() {
                        Some("bold") => st.bold = true,
                        Some("italic") => st.italic = true,
                        Some("underlined") => st.underline = true,
                        _ => {}
                    }
                }
            }
            v => st.fg = color(v),
        }
        scopes.push(k);
        styles.push(st);
    }
    (scopes, styles)
}

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("usage: highlight-spike <file> [theme]");
    let (scopes, styles) = theme(&args.next().unwrap_or("default".into()));
    let loader = loader(&scopes);
    let ext = path.rsplit('.').next().unwrap_or("");
    let lang = loader.langs.iter().position(|l| l.file_types.iter().any(|f| f == ext)).expect("no language for file");
    let text = Rope::from(std::fs::read_to_string(&path).unwrap());

    let t = std::time::Instant::now();
    let syntax = Syntax::new(text.slice(..), Language::new(lang as u32), Duration::from_millis(500), &loader).unwrap();
    let mut hl = Highlighter::new(&syntax, text.slice(..), &loader, ..);
    let mut out = String::new();
    let mut pos = 0u32;
    let mut style = Style::default();
    let end = text.len_bytes() as u32;
    while pos < end {
        let next = hl.next_event_offset().min(end);
        out += &style.ansi();
        out.extend(text.byte_slice(pos as usize..next as usize).chunks());
        pos = next;
        if pos >= end { break }
        let (event, active) = hl.advance();
        let base = if event == HighlightEvent::Refresh { Style::default() } else { style };
        style = active.fold(base, |s, h| s.patch(styles[h.idx()]));
    }
    print!("{out}\x1b[0m");
    eprintln!("[{} in {:?}]", loader.langs[lang].name, t.elapsed());
}

#[test]
fn inherits_resolve() {
    // typescript's highlights.scm only says `; inherits: ecma,_typescript`.
    let ts = query("typescript", "highlights.scm");
    assert!(!ts.contains("inherits: ecma") && ts.contains("@keyword"));
    // catppuccin_frappe inherits catppuccin_mocha's keys but overrides its palette.
    let (scopes, styles) = theme("catppuccin_frappe");
    let kw = styles[scopes.iter().position(|s| s == "keyword").unwrap()];
    assert_eq!(kw.fg, Some((0xca, 0x9e, 0xe6))); // frappe mauve, not mocha's #cba6f7
    // Every embedded language's queries compile against its grammar.
    assert!(loader(&scopes).langs.len() >= 2);
}
