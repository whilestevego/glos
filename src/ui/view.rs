//! One file as glos shows it, and drawing its text: the full-file view with Helix's gutters, and
//! the picker's preview without them.

use std::borrow::Cow;

use helix_core::{
    Rope, RopeSlice, Selection,
    doc_formatter::{DocumentFormatter, TextFormat},
    graphemes::Grapheme,
    text_annotations::TextAnnotations,
};
use helix_stdx::rope::RopeSliceExt;
use ratatui::{buffer::Buffer, layout::Rect, style::Style};
use unicode_width::UnicodeWidthStr;

use crate::{
    diff::{self, Hunk},
    source::{self, Change, Content, Kind, Mode as FileMode, Source},
    syntax::Loader,
    theme::Theme,
};

use super::{commands::SCROLLOFF, comments::Threads};

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
    /// Line numbers and diff signs; the comment box has none.
    pub gutter: bool,
}

impl File {
    pub fn change(change: &Change, source: &Source, loader: &Loader) -> File {
        let read = |c: &Content| source.read(c).ok().flatten().unwrap_or_default();
        let (old, new) = (read(&change.old), read(&change.new));
        let mut notices = vec![];
        match change.kind() {
            Kind::Added => notices.push("new file".to_string()),
            Kind::Deleted => notices.push("deleted file; showing what was removed".to_string()),
            Kind::Renamed => notices.push(format!(
                "renamed from {}",
                change.old_path.as_deref().unwrap_or("?")
            )),
            Kind::ModeChanged | Kind::Modified => {}
        }
        if let (Some(a), Some(b)) = (change.old_mode, change.new_mode)
            && a != b
        {
            notices.push(format!("mode changed: {} → {}", mode_name(a), mode_name(b)));
        }
        if change.new_mode == Some(FileMode::Symlink) || change.old_mode == Some(FileMode::Symlink)
        {
            notices.push("symlink; showing its target".to_string());
        }
        // A deleted file shows what was there.
        let shown = if change.new == Content::None {
            &old
        } else {
            &new
        };
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
            diff::hunks(
                &String::from_utf8_lossy(&old),
                &String::from_utf8_lossy(&new),
            )
        };
        let mut file = File::new(
            &source.root.join(&change.path),
            shown,
            binary,
            hunks,
            notices,
            loader,
        );
        file.collapsed = collapsed;
        // Open on the first change, as a reviewer wants.
        if let Some(h) = file.hunks.first() {
            let line = (h.after.start as usize).min(file.text.len_lines().saturating_sub(1));
            // A one-grapheme cursor, as Helix's set_selection makes it.
            file.sel = Selection::point(file.text.line_to_char(line))
                .ensure_invariants(file.text.slice(..));
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
        let text = if binary {
            Rope::new()
        } else {
            Rope::from(String::from_utf8_lossy(bytes).as_ref())
        };
        let lang = loader.language_for(path, text.slice(..));
        let syntax = lang.and_then(|l| loader.parse(text.slice(..), l));
        // A one-grapheme cursor, as Helix's set_selection makes it.
        let sel = Selection::point(0).ensure_invariants(text.slice(..));
        File {
            tab_width: loader.tab_width(lang),
            text,
            hunks,
            sel,
            top: 0,
            left: 0,
            syntax,
            notices,
            collapsed: false,
            binary,
            gutter: true,
        }
    }

    /// A buffer for writing in (the comment box): Markdown, with no gutter.
    pub fn scratch(text: &str, loader: &Loader) -> File {
        let mut f = File::new(
            std::path::Path::new("comment.md"),
            text.as_bytes(),
            false,
            vec![],
            vec![],
            loader,
        );
        f.gutter = false;
        f
    }

    /// Highlights the text again after it changed.
    pub fn reparse(&mut self, loader: &Loader) {
        let text = self.text.slice(..);
        let lang = loader.language_for(std::path::Path::new("comment.md"), text);
        self.syntax = lang.and_then(|l| loader.parse(text, l));
    }

    /// Helix's default gutters: a diagnostics column (glos's thread markers), a spacer, line
    /// numbers (at least 3 wide), a spacer and the diff signs.
    pub fn gutter_width(&self) -> usize {
        if !self.gutter {
            return 0;
        }
        1 + 1 + self.text.len_lines().to_string().len().max(3) + 1 + 1
    }
}

/// How `draw` styles a selection: (selection, primary selection, cursor, primary cursor).
pub type SelectionStyles = (Style, Style, Style, Style);

/// The text layout for a view `width` columns wide, with Helix's soft-wrap defaults
/// (helix-view `Document::text_format`): wrap indicator `↪ `, max-wrap 20, max-indent-retain 40.
pub fn text_format(file: &File, width: u16, wrap: bool) -> TextFormat {
    TextFormat {
        soft_wrap: wrap && width > 10,
        tab_width: file.tab_width as u16,
        max_wrap: 20.min(width / 4),
        max_indent_retain: 40.min(width * 2 / 5),
        wrap_indicator: "↪ ".into(),
        wrap_indicator_highlight: None,
        viewport_width: width,
        soft_wrap_at_text_width: false,
    }
}

/// Draws `file`'s notices and text into `area`, from line `top`: with the gutter and the
/// selection for the main view, without them for a preview. Lines are laid out by helix-core's
/// `DocumentFormatter`, so tabs, wide characters and soft wrap fall where Helix puts them.
/// Threads are drawn under their lines; returns the screen row of each thread row drawn, with
/// its thread.
#[allow(clippy::too_many_arguments)]
pub fn draw(
    buf: &mut Buffer,
    area: Rect,
    file: &File,
    top: usize,
    left: usize,
    wrap: bool,
    loader: &Loader,
    theme: &Theme,
    main: Option<SelectionStyles>,
    threads: &Threads,
) -> Vec<(u16, usize)> {
    let mut hits = vec![];
    let mut y = area.y;
    for notice in &file.notices {
        if y >= area.bottom() {
            return hits;
        }
        buf.set_stringn(
            area.x + 1,
            y,
            clean(notice),
            area.width.saturating_sub(1) as usize,
            theme.get("ui.virtual"),
        );
        y += 1;
    }
    if file.collapsed || file.binary {
        return hits;
    }
    let text = file.text.slice(..);
    let height = area.bottom().saturating_sub(y) as usize;
    let top = top.min(text.len_lines().saturating_sub(1));
    let last = (top + height).min(text.len_lines());
    let spans = match &file.syntax {
        Some(s) => loader.highlights(
            s,
            text,
            text.line_to_byte(top)..text.line_to_byte(last),
            theme,
        ),
        None => vec![],
    };
    let mut span = 0;

    let gutter = if main.is_some() {
        file.gutter_width() as u16
    } else {
        0
    };
    let digits = text.len_lines().to_string().len().max(3);
    let cursor_lines: Vec<usize> = file
        .sel
        .iter()
        .map(|r| text.char_to_line(r.cursor(text)))
        .collect();
    let primary = file.sel.primary_index();
    let ranges: Vec<_> = match main {
        Some(_) => file
            .sel
            .iter()
            .map(|r| (r.from(), r.to(), r.cursor(text)))
            .collect(),
        None => vec![],
    };
    let text_style = theme.get("ui.text");
    let wrap_style = theme.get("ui.virtual.wrap");
    let (sel_style, sel_primary, cursor, cursor_primary) = main.unwrap_or_default();
    let (added, deleted, modified) = (
        theme.get("diff.plus.gutter"),
        theme.get("diff.minus.gutter"),
        theme.get("diff.delta.gutter"),
    );

    let x0 = area.x + gutter;
    let width = area.width.saturating_sub(gutter);
    let fmt = text_format(file, width, wrap);
    let left = if fmt.soft_wrap { 0 } else { left };
    let annotations = TextAnnotations::default();
    let bottom = area.bottom();
    let mut hunk_i = 0;
    let mut line = top;
    while y < bottom && line < text.len_lines() {
        // The diff sign for this line, as Helix's diff gutter picks it.
        let l = line as u32;
        while let Some(h) = file.hunks.get(hunk_i) {
            if h.after.end < l || (!h.after.is_empty() && l == h.after.end) {
                hunk_i += 1;
            } else {
                break;
            }
        }
        let sign = file
            .hunks
            .get(hunk_i)
            .filter(|h| h.after.start <= l)
            .map(|h| {
                if h.before.is_empty() {
                    ("▍", added, true)
                } else if h.after.is_empty() {
                    ("▔", deleted, false) // a removal shows on the first row only
                } else {
                    ("▍", modified, true)
                }
            });
        let mark = threads
            .marks
            .iter()
            .find(|(s, e, _)| *s <= line && line <= *e)
            .map(|m| m.2);
        let gutter_row = |buf: &mut Buffer, row: usize, y: u16| {
            if main.is_none() || gutter == 0 {
                return;
            }
            if row == 0 {
                // The diagnostics column: a thread on this line.
                if let Some(style) = mark {
                    buf.set_string(area.x, y, "●", style);
                }
                let selected = cursor_lines.contains(&line);
                let nr_style = theme.get(if selected {
                    "ui.linenr.selected"
                } else {
                    "ui.linenr"
                });
                buf.set_string(area.x + 2, y, format!("{:>digits$}", line + 1), nr_style);
            }
            if let Some((sign, style, every_row)) = sign
                && (row == 0 || every_row)
            {
                buf.set_string(x0 - 1, y, sign, style);
            }
        };
        gutter_row(buf, 0, y);
        let mut rows = 1;
        let mut end_col = 0;
        let mut end_char = text.line_to_char(line);
        let start = end_char;
        for g in DocumentFormatter::new_at_prev_checkpoint(text, &fmt, &annotations, start) {
            if g.line_idx != line {
                break;
            }
            let row = g.visual_pos.row;
            let yy = y + row as u16;
            if yy >= bottom {
                break;
            }
            if row + 1 > rows {
                gutter_row(buf, row, yy);
                rows = row + 1;
            }
            let col = g.visual_pos.col;
            let w = g.raw.width().max(1);
            if g.is_virtual() {
                // The wrap indicator.
                if col >= left && col + w <= left + width as usize {
                    buf.set_string(
                        x0 + (col - left) as u16,
                        yy,
                        g.raw.to_string(),
                        text_style.patch(wrap_style),
                    );
                }
                continue;
            }
            let ch = g.char_idx;
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
            if col >= left && col + w <= left + width as usize {
                let x = x0 + (col - left) as u16;
                match &g.raw {
                    Grapheme::Other { g: s } => {
                        buf[(x, yy)].set_symbol(&clean(s)).set_style(style);
                        for i in 1..w as u16 {
                            buf[(x + i, yy)].set_symbol("").set_style(style);
                        }
                    }
                    // Tabs and the newline (where the cursor can sit) draw as spaces.
                    _ => {
                        for i in 0..w as u16 {
                            buf[(x + i, yy)].set_symbol(" ").set_style(style);
                        }
                    }
                }
            }
            end_col = col + w;
            end_char = ch + g.doc_chars();
            if matches!(g.raw, Grapheme::Newline) {
                break;
            }
        }
        // The cursor can sit at the very end of a file with no trailing newline.
        if end_char == text.len_chars() && ranges.iter().any(|r| r.2 == end_char) {
            let row_y = y + rows as u16 - 1;
            if end_col >= left && end_col < left + width as usize && row_y < bottom {
                buf[(x0 + (end_col - left) as u16, row_y)]
                    .set_symbol(" ")
                    .set_style(cursor_primary);
            }
        }
        y += rows as u16;
        for r in threads.rows.get(&line).into_iter().flatten() {
            if y >= bottom {
                break;
            }
            buf.set_stringn(x0, y, clean(&r.text), width as usize, r.style);
            hits.push((y, r.thread));
            y += 1;
        }
        line += 1;
    }
    hits
}

/// The display column of char `pos` on its line, with tabs expanded.
pub fn visual_col(text: RopeSlice, pos: usize, tab_width: usize) -> usize {
    let start = text.line_to_char(text.char_to_line(pos));
    let mut col = 0;
    for g in text.slice(start..pos).graphemes() {
        let g: Cow<str> = g.into();
        col += if g == "\t" {
            tab_width - col % tab_width
        } else {
            clean(&g).width().max(1)
        };
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

    use crate::{
        source::Source,
        theme::{ColorSupport, Theme},
    };

    /// Draws `text` as a new file into a `w`×`h` buffer, from column `left`, and returns the
    /// rows as text.
    fn drawn(text: &str, w: u16, h: u16, left: usize, wrap: bool, main: bool) -> Vec<String> {
        let dir = tempfile::tempdir().unwrap();
        let (old, new) = (dir.path().join("a.txt"), dir.path().join("b.txt"));
        std::fs::write(&old, "").unwrap();
        std::fs::write(&new, text).unwrap();
        let source = Source::diff(&old, &new).unwrap();
        let theme = Theme::load("default", ColorSupport::TrueColor).unwrap();
        let loader = Loader::new(&theme).unwrap();
        let mut file = File::change(&source.changes[0], &source, &loader);
        file.notices.clear();
        let area = Rect::new(0, 0, w, h);
        let mut buf = Buffer::empty(area);
        let styles = main.then(SelectionStyles::default);
        draw(
            &mut buf,
            area,
            &file,
            0,
            left,
            wrap,
            &loader,
            &theme,
            styles,
            &Threads::default(),
        );
        (0..h)
            .map(|y| {
                (0..w)
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect()
    }

    #[test]
    fn tabs_wide_and_control_characters() {
        // Tabs to the next stop of 4; 漢 takes two cells; ESC shows as ␛.
        let rows = drawn("\tx\n漢字y\na\x1bb\n", 20, 3, 0, false, false);
        assert_eq!(rows, ["    x", "漢字y", "a␛b"]);
    }

    #[test]
    fn horizontal_offset_never_splits_a_wide_character() {
        let rows = drawn("漢字abc\n", 20, 1, 1, false, false);
        // Column 1 is the second half of 漢: it's skipped, and 字 starts the row.
        assert_eq!(rows, [" 字abc"]);
        let rows = drawn("abcdef\n", 20, 1, 3, false, false);
        assert_eq!(rows, ["def"]);
    }

    #[test]
    fn main_view_has_a_gutter_and_preview_does_not() {
        let rows = drawn("a\nb\n", 20, 2, 0, false, true);
        assert_eq!(rows, ["    1 ▍a", "    2 ▍b"]);
        let rows = drawn("a\nb\n", 20, 2, 0, false, false);
        assert_eq!(rows, ["a", "b"]);
    }

    #[test]
    fn long_lines_clip_or_wrap() {
        let line = "one two three four five six\n";
        assert_eq!(
            drawn(line, 12, 3, 0, false, false),
            ["one two thre", "", ""]
        );
        let rows = drawn(line, 12, 4, 0, true, false);
        assert!(rows[0].starts_with("one two"), "{rows:?}");
        assert!(rows[1].starts_with("↪ "), "{rows:?}");
        assert!(rows.concat().contains("six"), "{rows:?}");
    }

    #[test]
    fn notices_come_first_and_collapsed_files_show_only_them() {
        let dir = tempfile::tempdir().unwrap();
        let (old, new) = (dir.path().join("o"), dir.path().join("Cargo.lock"));
        std::fs::write(&old, "").unwrap();
        std::fs::write(&new, "x\n").unwrap();
        let source = Source::diff(&old, &new).unwrap();
        let theme = Theme::load("default", ColorSupport::TrueColor).unwrap();
        let loader = Loader::new(&theme).unwrap();
        let mut file = File::change(&source.changes[0], &source, &loader);
        // Two files given directly aren't "generated", so collapse by hand.
        file.collapsed = true;
        file.notices = vec!["collapsed".into()];
        let area = Rect::new(0, 0, 20, 3);
        let mut buf = Buffer::empty(area);
        draw(
            &mut buf,
            area,
            &file,
            0,
            0,
            false,
            &loader,
            &theme,
            None,
            &Threads::default(),
        );
        let row = |y| {
            (0..20)
                .map(|x| buf[(x, y)].symbol())
                .collect::<String>()
                .trim_end()
                .to_string()
        };
        assert_eq!([row(0), row(1)], [" collapsed".to_string(), String::new()]);
    }
}
