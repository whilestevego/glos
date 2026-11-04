//! Syntax highlighting: Helix's `languages.toml` and queries with grammars compiled into glos,
//! through Helix's highlighter (`tree-house`).
//!
//! Language matching follows helix-core's `Loader` (globs, extensions, shebangs, injection
//! regexes); grammars come from `embedded::GRAMMARS` instead of shared libraries on disk. Each
//! language's queries compile the first time it's used: compiling all of them takes ~260 ms.

use std::{borrow::Cow, collections::HashMap, ops::Range, path::Path, sync::OnceLock, time::Duration};

use anyhow::Result;
use helix_core::syntax::config::{Configuration, FileType, LanguageConfiguration};
use helix_stdx::rope::RopeSliceExt;
use ratatui::style::Style;
use ropey::RopeSlice;
use tree_house::{
    InjectionLanguageMarker, Language, LanguageConfig, LanguageLoader, Syntax,
    highlighter::{Highlight, HighlightEvent, Highlighter},
    tree_sitter::Grammar,
};

use crate::{embedded, theme::Theme};

const PARSE_TIMEOUT: Duration = Duration::from_millis(500); // as Helix

struct LangData {
    config: LanguageConfiguration,
    grammar: Option<unsafe extern "C" fn() -> *const ()>,
    compiled: OnceLock<Option<LanguageConfig>>,
}

pub struct Loader {
    langs: Vec<LangData>,
    by_extension: HashMap<String, Language>,
    by_shebang: HashMap<String, Language>,
    globs: globset::GlobSet,
    glob_langs: Vec<(usize, Language)>,
    /// Theme scopes, in highlight-index order.
    scopes: Vec<String>,
}

impl Loader {
    pub fn new(theme: &Theme) -> Result<Loader> {
        let config: Configuration = toml::from_str(embedded::LANGUAGES_TOML)?;
        let mut loader = Loader {
            langs: vec![],
            by_extension: HashMap::new(),
            by_shebang: HashMap::new(),
            globs: globset::GlobSet::empty(),
            glob_langs: vec![],
            scopes: theme.scopes().to_vec(),
        };
        let mut globs = globset::GlobSetBuilder::new();
        for config in config.language {
            let lang = Language::new(loader.langs.len() as u32);
            for ft in &config.file_types {
                match ft {
                    FileType::Extension(e) => {
                        loader.by_extension.insert(e.clone(), lang);
                    }
                    FileType::Glob(g) => {
                        globs.add(g.clone());
                        loader.glob_langs.push((g.glob().len(), lang));
                    }
                }
            }
            for s in &config.shebangs {
                loader.by_shebang.insert(s.clone(), lang);
            }
            let name = config.grammar.as_deref().unwrap_or(&config.language_id);
            let grammar = embedded::GRAMMARS.iter().find(|(g, _)| *g == name).map(|(_, f)| *f);
            loader.langs.push(LangData { config, grammar, compiled: OnceLock::new() });
        }
        loader.globs = globs.build()?;
        Ok(loader)
    }

    /// The language for a file, by path (globs, then extension) and then by shebang.
    pub fn language_for(&self, path: &Path, text: RopeSlice) -> Option<Language> {
        self.language_for_path(path).or_else(|| self.language_for_shebang(text))
    }

    fn language_for_path(&self, path: &Path) -> Option<Language> {
        self.globs
            .matches(path)
            .iter()
            .map(|i| self.glob_langs[*i])
            .max_by_key(|(len, _)| *len)
            .map(|(_, l)| l)
            .or_else(|| self.by_extension.get(path.extension()?.to_str()?).copied())
    }

    fn language_for_shebang(&self, text: RopeSlice) -> Option<Language> {
        use std::sync::LazyLock;
        // helix-core's, anchored at the start of the file.
        static SHEBANG: LazyLock<helix_stdx::rope::Regex> = LazyLock::new(|| {
            helix_stdx::rope::Regex::new(r"^#!\s*(?:\S*[/\\](?:env\s+(?:\-\S+\s+)*)?)?([^\s\.\d]+)")
                .unwrap()
        });
        let first = text.slice(..text.len_chars().min(256));
        let cap = SHEBANG.captures_iter(first.regex_input()).next()?;
        let marker: Cow<str> = first.byte_slice(cap.get_group(1)?.range()).into();
        self.by_shebang.get(marker.as_ref()).copied()
    }

    fn language_for_name(&self, name: &str) -> Option<Language> {
        let i = self.langs.iter().position(|l| l.config.language_id == name)?;
        Some(Language::new(i as u32))
    }

    pub fn name(&self, lang: Language) -> &str {
        &self.langs[lang.idx()].config.language_id
    }

    /// The language's tab width (`indent.tab-width`), 4 if it sets none, as Helix.
    pub fn tab_width(&self, lang: Option<Language>) -> usize {
        lang.and_then(|l| self.langs[l.idx()].config.indent.as_ref()).map_or(4, |i| i.tab_width)
    }

    pub fn parse(&self, text: RopeSlice, lang: Language) -> Option<Syntax> {
        self.get_config(lang)?;
        Syntax::new(text, lang, PARSE_TIMEOUT, self).ok()
    }

    /// Syntax styles over `bytes`: each entry is a byte offset and the style from there on.
    pub fn highlights(
        &self,
        syntax: &Syntax,
        text: RopeSlice,
        bytes: Range<usize>,
        theme: &Theme,
    ) -> Vec<(usize, Style)> {
        let end = bytes.end as u32;
        let mut hl = Highlighter::new(syntax, text, self, bytes.start as u32..end);
        let mut out = vec![(bytes.start, Style::default())];
        let mut style = Style::default();
        loop {
            let pos = hl.next_event_offset();
            if pos >= end {
                break;
            }
            let (event, active) = hl.advance();
            let base = if event == HighlightEvent::Refresh { Style::default() } else { style };
            style = active.fold(base, |s, h: Highlight| s.patch(theme.highlight(h.idx())));
            out.push((pos as usize, style));
        }
        out
    }
}

impl LanguageLoader for Loader {
    fn language_for_marker(&self, marker: InjectionLanguageMarker) -> Option<Language> {
        match marker {
            InjectionLanguageMarker::Name(name) => self.language_for_name(name),
            InjectionLanguageMarker::Match(text) => {
                // As helix-core: an exact name first, then the longest injection-regex match.
                let name: Cow<str> = text.into();
                self.language_for_name(&name).or_else(|| {
                    self.langs
                        .iter()
                        .enumerate()
                        .filter_map(|(i, l)| {
                            let m = l.config.injection_regex.as_ref()?.find(text.regex_input())?;
                            Some((m.end() - m.start(), i))
                        })
                        .max_by_key(|(len, _)| *len)
                        .map(|(_, i)| Language::new(i as u32))
                })
            }
            InjectionLanguageMarker::Filename(text) => {
                let name: Cow<str> = text.into();
                self.language_for_path(Path::new(name.as_ref()))
            }
            InjectionLanguageMarker::Shebang(text) => {
                let name: Cow<str> = text.into();
                self.by_shebang.get(name.as_ref()).copied()
            }
        }
    }

    fn get_config(&self, lang: Language) -> Option<&LanguageConfig> {
        let data = self.langs.get(lang.idx())?;
        data.compiled
            .get_or_init(|| {
                let f = data.grammar?;
                // SAFETY: each GRAMMARS entry is a tree-sitter language function compiled in
                // by build.rs, which is what LanguageFn wraps.
                let f = unsafe { tree_sitter_language::LanguageFn::from_raw(std::mem::transmute(f)) };
                let grammar = Grammar::try_from(f).ok()?;
                let name = &data.config.language_id;
                let config = LanguageConfig::new(
                    grammar,
                    &query(name, "highlights.scm"),
                    &query(name, "injections.scm"),
                    &query(name, "locals.scm"),
                )
                .ok()?;
                configure(&config, &self.scopes);
                Some(config)
            })
            .as_ref()
    }
}

/// A query with its `; inherits:` resolved, as Helix's `read_query`.
fn query(lang: &str, file: &str) -> String {
    tree_house::read_query(lang, |l| {
        embedded::QUERIES
            .iter()
            .find(|(ql, qf, _)| *ql == l && *qf == file)
            .map(|q| q.2.to_string())
            .unwrap_or_default()
    })
}

/// Helix's scope matching: a capture takes the theme scope that is its longest prefix by
/// segments (`keyword.control.import` → `keyword.control` → `keyword`).
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::ColorSupport;
    use ropey::Rope;

    fn loader() -> (Loader, Theme) {
        let theme = Theme::load("catppuccin_mocha", ColorSupport::TrueColor).unwrap();
        (Loader::new(&theme).unwrap(), theme)
    }

    #[test]
    fn finds_languages() {
        let (l, _) = loader();
        let empty = Rope::new();
        let name = |p: &str| l.language_for(Path::new(p), empty.slice(..)).map(|x| l.name(x).to_string());
        assert_eq!(name("/r/src/main.rs").as_deref(), Some("rust"));
        assert_eq!(name("/r/a.tsx").as_deref(), Some("tsx"));
        assert_eq!(name("/r/Cargo.lock").as_deref(), Some("toml"));
        let script = Rope::from("#!/usr/bin/env python3\nprint(1)\n");
        assert_eq!(l.language_for(Path::new("/r/run"), script.slice(..)).map(|x| l.name(x)), Some("python"));
        assert_eq!(l.tab_width(name("/r/a.go").and_then(|n| l.language_for_name(&n))), 4);
    }

    #[test]
    fn highlights_with_inherited_queries_and_injections() {
        let (l, theme) = loader();
        // TypeScript's highlights.scm is only `; inherits: ecma,_typescript`.
        let text = Rope::from("const x: number = 1;\n");
        let ts = l.language_for(Path::new("/a.ts"), text.slice(..)).unwrap();
        let syntax = l.parse(text.slice(..), ts).unwrap();
        let spans = l.highlights(&syntax, text.slice(..), 0..text.len_bytes(), &theme);
        let keyword = theme.get("keyword");
        assert!(spans.iter().any(|(at, s)| *at == 0 && s.fg == keyword.fg), "{spans:?}");

        // Markdown injects rust into a fenced block.
        let md = Rope::from("# T\n\n```rust\nfn main() {}\n```\n");
        let lang = l.language_for(Path::new("/a.md"), md.slice(..)).unwrap();
        let syntax = l.parse(md.slice(..), lang).unwrap();
        let fn_at = md.to_string().find("fn").unwrap();
        let spans = l.highlights(&syntax, md.slice(..), 0..md.len_bytes(), &theme);
        let at_fn = spans.iter().rev().find(|(at, _)| *at <= fn_at).unwrap().1;
        assert_eq!(at_fn.fg, theme.get("keyword.function").fg.or(keyword.fg));
    }
}
