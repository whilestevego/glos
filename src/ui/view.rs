//! One file as glos shows it, and drawing its text: the full-file view with Helix's gutters, and
//! the picker's preview without them.

use std::borrow::Cow;

use helix_core::{Rope, RopeSlice, Selection};
use helix_stdx::rope::RopeSliceExt;
use ratatui::{buffer::Buffer, layout::Rect, style::Style};
use unicode_width::UnicodeWidthStr;

use crate::{
    diff::{self, Hunk},
    source::{self, Change, Content, Kind, Mode as FileMode, Source},
    syntax::Loader,
    theme::Theme,
};

use super::commands::SCROLLOFF;

/// A file loaded for viewing: a changed file of the review, or any file opened for context.
pub struct File {
    pub text: Rope,
    pub hunks: Vec<Hunk>,
    pub sel: Selection,
    pub top: usize,
    pub left: usize,
    pub syntax: Option<tree_house::Syntax>,
    pub tab_width: usize,
    /// One-line headers above the text: the kind of change, binary, collapsed…
    pub notices: Vec<String>,
    /// Lockfiles and generated files start collapsed; `enter` expands them.
    pub collapsed: bool,
    /// Binary files have no text to show.
    pub binary: bool,
}

impl File {
    pub fn change(change: &Change, source: &Source, loader: &Loader) -> File {
        let read = |c: &Content| source.read(c).ok().flatten().unwrap_or_default();
        let (old, new) = (read(&change.old), read(&change.new));
        let mut notices = vec![];
        match change.kind() {
            Kind::Added => notices.push("new file".to_string()),
            Kind::Deleted => notices.push("deleted file; showing what was removed".to_string()),
            Kind::Renamed => notices.push(format!("renamed from {}", change.old_path.as_deref().unwrap_or("?"))),
            Kind::ModeChanged | Kind::Modified => {}
        }
        if let (Some(a), Some(b)) = (change.old_mode, change.new_mode) {
            if a != b {
                notices.push(format!("mode changed: {} → {}", mode_name(a), mode_name(b)));
            }
        }
        if change.new_mode == Some(FileMode::Symlink) || change.old_mode == Some(FileMode::Symlink) {
            notices.push("symlink; showing its target".to_string());
        }
        // A deleted file shows what was there.
        let shown = if change.new == Content::None { &old } else { &new };
        let binary = source::is_binary(&old) || source::is_binary(shown);
        if binary {
            notices.push("binary file; not shown".to_string());
        }
        let collapsed = source.collapsed(change);
        if collapsed {
            notices.push("generated file, collapsed; enter expands".to_string());
        }
        let hunks = if binary || change.new == Content::None {
            vec![]
        } else {
            diff::hunks(&String::from_utf8_lossy(&old), &String::from_utf8_lossy(&new))
        };
        let mut file = File::new(&source.root.join(&change.path), shown, binary, hunks, notices, loader);
        file.collapsed = collapsed;
        // Open on the first change, as a reviewer wants.
        if let Some(h) = file.hunks.first() {
            let line = (h.after.start as usize).min(file.text.len_lines().saturating_sub(1));
            file.sel = Selection::point(file.text.line_to_char(line));
            file.top = line.saturating_sub(SCROLLOFF);
        }
        file
    }

    /// Any file in the repo, opened read-only for context.
    pub fn context(path: &str, source: &Source, loader: &Loader) -> File {
        let full = source.root.join(path);
        let (bytes, mut notices) = match std::fs::read(&full) {
            Ok(b) => (b, vec!["not changed; shown for context".to_string()]),
            Err(e) => (vec![], vec![format!("can't read {path}: {e}")]),
        };
        let binary = source::is_binary(&bytes);
        if binary {
            notices.push("binary file; not shown".to_string());
        }
        File::new(&full, &bytes, binary, vec![], notices, loader)
    }

    fn new(
        path: &std::path::Path,
        bytes: &[u8],
        binary: bool,
        hunks: Vec<Hunk>,
        notices: Vec<String>,
        loader: &Loader,
    ) -> File {
        let text = if binary { Rope::new() } else { Rope::from(String::from_utf8_lossy(bytes).as_ref()) };
        let lang = loader.language_for(path, text.slice(..));
        let syntax = lang.and_then(|l| loader.parse(text.slice(..), l));
        File {
            tab_width: loader.tab_width(lang),
            text,
            hunks,
            sel: Selection::point(0),
            top: 0,
            left: 0,
            syntax,
            notices,
            collapsed: false,
            binary,
        }
    }

    /// Helix's default gutters: a diagnostics column (glos's thread markers), a spacer, line
    /// numbers (at least 3 wide), a spacer and the diff signs.
    pub fn gutter_width(&self) -> usize {
        1 + 1 + self.text.len_lines().to_string().len().max(3) + 1 + 1
    }
}

/// How `draw` styles a selection: (selection, primary selection, cursor, primary cursor).
pub type SelectionStyles = (Style, Style, Style, Style);

/// Draws `file`'s notices and text into `area`, from line `top`: with the gutter and the
/// selection for the main view, without them for a preview.
pub fn draw(
    buf: &mut Buffer,
    area: Rect,
    file: &File,
    top: usize,
    left: usize,
    loader: &Loader,
    theme: &Theme,
    main: Option<SelectionStyles>,
) {
    let mut y = area.y;
    for notice in &file.notices {
        if y >= area.bottom() {
            return;
        }
        buf.set_stringn(area.x + 1, y, clean(notice), area.width.saturating_sub(1) as usize, theme.get("ui.virtual"));
        y += 1;
    }
    if file.collapsed || file.binary {
        return;
    }
    let text = file.text.slice(..);
    let height = area.bottom().saturating_sub(y) as usize;
    let top = top.min(text.len_lines().saturating_sub(1));
    let last = (top + height).min(text.len_lines());
    let spans = match &file.syntax {
        Some(s) => loader.highlights(s, text, text.line_to_byte(top)..text.line_to_byte(last), theme),
        None => vec![],
    };
    let mut span = 0;

    let gutter = if main.is_some() { file.gutter_width() as u16 } else { 0 };
    let digits = text.len_lines().to_string().len().max(3);
    let cursor_lines: Vec<usize> = file.sel.iter().map(|r| text.char_to_line(r.cursor(text))).collect();
    let primary = file.sel.primary_index();
    let ranges: Vec<_> = match main {
        Some(_) => file.sel.iter().map(|r| (r.from(), r.to(), r.cursor(text))).collect(),
        None => vec![],
    };
    let text_style = theme.get("ui.text");
    let (sel_style, sel_primary, cursor, cursor_primary) = main.unwrap_or_default();
    let (added, deleted, modified) =
        (theme.get("diff.plus.gutter"), theme.get("diff.minus.gutter"), theme.get("diff.delta.gutter"));

    let x0 = area.x + gutter;
    let width = area.width.saturating_sub(gutter) as usize;
    let mut hunk_i = 0;
    for line in top..last {
        if main.is_some() {
            // The line number, then the diff sign as Helix's diff gutter draws it.
            let selected = cursor_lines.contains(&line);
            let nr_style = theme.get(if selected { "ui.linenr.selected" } else { "ui.linenr" });
            buf.set_string(area.x + 2, y, format!("{:>digits$}", line + 1), nr_style);
            let l = line as u32;
            while let Some(h) = file.hunks.get(hunk_i) {
                if h.after.end < l || (!h.after.is_empty() && l == h.after.end) {
                    hunk_i += 1;
                } else {
                    break;
                }
            }
            if let Some(h) = file.hunks.get(hunk_i).filter(|h| h.after.start <= l) {
                let (sign, style) = if h.before.is_empty() {
                    ("▍", added)
                } else if h.after.is_empty() {
                    ("▔", deleted)
                } else {
                    ("▍", modified)
                };
                buf.set_string(x0 - 1, y, sign, style);
            }
        }

        let mut col = 0usize;
        let mut ch = text.line_to_char(line);
        for g in text.line(line).graphemes() {
            let g: Cow<str> = g.into();
            let newline = g.ends_with('\n');
            let byte = text.char_to_byte(ch);
            while span + 1 < spans.len() && spans[span + 1].0 <= byte {
                span += 1;
            }
            let mut style = text_style;
            if let Some((_, s)) = spans.get(span) {
                style = style.patch(*s);
            }
            for (i, (from, to, cur)) in ranges.iter().enumerate() {
                if *from <= ch && ch < *to {
                    style = style.patch(if i == primary { sel_primary } else { sel_style });
                }
                if *cur == ch {
                    style = style.patch(if i == primary { cursor_primary } else { cursor });
                }
            }
            let (symbol, w): (Cow<str>, usize) = if newline {
                (" ".into(), 1)
            } else if g == "\t" {
                (" ".into(), file.tab_width - col % file.tab_width)
            } else {
                let s = clean(&g);
                let w = s.width().max(1);
                (s, w)
            };
            if col >= left && col + w <= left + width {
                let x = x0 + (col - left) as u16;
                if g == "\t" || newline {
                    for i in 0..w as u16 {
                        buf[(x + i, y)].set_symbol(" ").set_style(style);
                    }
                } else {
                    buf[(x, y)].set_symbol(&symbol).set_style(style);
                    for i in 1..w as u16 {
                        buf[(x + i, y)].set_symbol("").set_style(style);
                    }
                }
            }
            col += w;
            ch += g.chars().count();
            if newline || col >= left + width {
                break;
            }
        }
        // The cursor can sit at the very end of a file with no trailing newline.
        if ch == text.len_chars() && ranges.iter().any(|r| r.2 == ch) && col >= left && col < left + width {
            buf[(x0 + (col - left) as u16, y)].set_symbol(" ").set_style(cursor_primary);
        }
        y += 1;
    }
}

/// The display column of char `pos` on its line, with tabs expanded.
pub fn visual_col(text: RopeSlice, pos: usize, tab_width: usize) -> usize {
    let start = text.line_to_char(text.char_to_line(pos));
    let mut col = 0;
    for g in text.slice(start..pos).graphemes() {
        let g: Cow<str> = g.into();
        col += if g == "\t" { tab_width - col % tab_width } else { clean(&g).width().max(1) };
    }
    col
}

/// Untrusted text is never drawn raw: control characters become visible symbols, so escape
/// sequences in code or agent comments can't reach the terminal.
pub fn clean(s: &str) -> Cow<'_, str> {
    if !s.chars().any(char::is_control) {
        return s.into();
    }
    s.chars()
        .map(|c| match c as u32 {
            0..=0x1f => char::from_u32(0x2400 + c as u32).unwrap(),
            0x7f => '␡',
            _ if c.is_control() => '�',
            _ => c,
        })
        .collect::<String>()
        .into()
}

fn mode_name(m: FileMode) -> &'static str {
    match m {
        FileMode::File => "file",
        FileMode::Executable => "executable",
        FileMode::Symlink => "symlink",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_characters_are_never_drawn() {
        assert_eq!(clean("plain"), "plain");
        assert_eq!(clean("a\x1b]0;pwned\x07b"), "a␛]0;pwned␇b");
        assert_eq!(clean("\u{9b}31m"), "�31m");
    }

    #[test]
    fn columns_expand_tabs_and_wide_chars() {
        let t = Rope::from("\tab漢x");
        assert_eq!(visual_col(t.slice(..), 1, 4), 4);
        assert_eq!(visual_col(t.slice(..), 4, 4), 8);
        assert_eq!(visual_col(t.slice(..), 5, 4), 9);
    }
}
