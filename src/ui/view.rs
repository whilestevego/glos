//! One file as glos shows it, and drawing its text: the full-file view with Helix's gutters, and
//! the picker's preview without them.

use std::borrow::Cow;

use glos_helix::{
    commands::SCROLLOFF,
    layout::{DocumentFormatter, TextAnnotations, TextFormat},
    syntax::{Loader, Syntax},
    text::{Grapheme, Rope, RopeSlice, RopeSliceExt, Selection},
};
use ratatui::{buffer::Buffer, layout::Rect, style::Style};
use unicode_width::UnicodeWidthStr;

use crate::{
    diff::{self, Hunk},
    source::{self, Change, Content, Kind, Mode as FileMode, Source},
    theme::Theme,
};

use super::comments::Threads;

/// A file loaded for viewing: a changed file of the review, or any file opened for context.
pub struct File {
    pub text: Rope,
    pub hunks: Vec<Hunk>,
    pub sel: Selection,
    pub top: usize,
    pub left: usize,
    pub syntax: Option<Syntax>,
    pub tab_width: usize,
    /// One-line headers above the text: the kind of change, binary, collapsed…
    pub notices: Vec<String>,
    /// Lockfiles and generated files start collapsed; `enter` expands them.
    pub collapsed: bool,
    /// Binary files have no text to show.
    pub binary: bool,
    /// Line numbers and diff signs; the comment box has none.
    pub gutter: bool,
    /// For the unified and side-by-side views: where each line came from. `None` for a file
    /// as it is.
    pub rows: Option<Vec<Row>>,
    /// Highlighting precomputed for such a view, from the two sides' syntax trees.
    pub spans: Option<Vec<(usize, Style)>>,
    /// The left side of a side-by-side view numbers its lines by the old file.
    pub old_numbers: bool,
    /// The biggest line number shown, for the gutter's width.
    pub numbers: usize,
}

/// Where a line of a unified or side-by-side view came from (0-based line numbers).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Row {
    Same {
        old: usize,
        new: usize,
    },
    Added(usize),
    Removed(usize),
    /// A line of the file as it is, in the whole-file view when something in it is folded.
    Line(usize),
    /// Rows folded away: from row `start` of the whole view, `len` of them, holding `changes`
    /// changed rows and `threads` threads (a fold you made can hide those), and the first and
    /// last lines they hide on each side, if any.
    Fold {
        start: usize,
        len: usize,
        changes: usize,
        threads: usize,
        lines: Option<(usize, usize)>,
        old_lines: Option<(usize, usize)>,
    },
    /// Padding that keeps the two sides of a side-by-side view level.
    Filler,
}

impl Row {
    /// The new side's line, for comments and threads.
    pub fn new_line(self) -> Option<usize> {
        match self {
            Row::Same { new, .. } | Row::Added(new) | Row::Line(new) => Some(new),
            _ => None,
        }
    }

    /// The old side's line.
    pub fn old_line(self) -> Option<usize> {
        match self {
            Row::Same { old, .. } | Row::Removed(old) => Some(old),
            _ => None,
        }
    }

    /// A fold's hidden lines as the gutter numbers them (1-based): `16–31`, or `16`.
    pub fn fold_range(self, old: bool) -> Option<String> {
        let Row::Fold {
            lines, old_lines, ..
        } = self
        else {
            return None;
        };
        let (a, b) = if old { old_lines } else { lines }?;
        Some(if a == b {
            format!("{}", a + 1)
        } else {
            format!("{}–{}", a + 1, b + 1)
        })
    }
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
            rows: None,
            spans: None,
            old_numbers: false,
            numbers: 0,
        }
        .numbered()
    }

    fn numbered(mut self) -> File {
        self.numbers = self.text.len_lines();
        self
    }

    /// A view made of lines from both sides (unified, or one side of side by side).
    pub fn synthetic(
        text: String,
        tab_width: usize,
        rows: Vec<Row>,
        spans: Vec<(usize, Style)>,
        numbers: usize,
    ) -> File {
        let text = Rope::from(text);
        let sel = Selection::point(0).ensure_invariants(text.slice(..));
        // Each run of added and removed lines is a change, in the view's own lines, so `]g`
        // and `[g` go through them as in the file view; so is a fold hiding changes.
        let mut hunks: Vec<Hunk> = vec![];
        for (i, r) in rows.iter().enumerate() {
            if !matches!(
                r,
                Row::Added(_) | Row::Removed(_) | Row::Fold { changes: 1.., .. }
            ) {
                continue;
            }
            let i = i as u32;
            match hunks.last_mut() {
                Some(h) if h.after.end == i => {
                    h.after.end = i + 1;
                    h.before.end = i + 1;
                }
                _ => hunks.push(Hunk {
                    before: i..i + 1,
                    after: i..i + 1,
                }),
            }
        }
        File {
            text,
            hunks,
            sel,
            top: 0,
            left: 0,
            syntax: None,
            tab_width,
            notices: vec![],
            collapsed: false,
            binary: false,
            gutter: true,
            rows: Some(rows),
            spans: Some(spans),
            old_numbers: false,
            numbers,
        }
    }

    /// The new side's line shown on view line `line`, if it is one.
    pub fn new_line_at(&self, line: usize) -> Option<usize> {
        match &self.rows {
            None => Some(line),
            Some(rows) => rows.get(line).and_then(|r| r.new_line()),
        }
    }

    /// The view line showing the new side's line `new` (or the next one shown after it).
    pub fn line_of(&self, new: usize) -> usize {
        match &self.rows {
            None => new,
            Some(rows) => rows
                .iter()
                .position(|r| r.new_line().is_some_and(|n| n >= new))
                .unwrap_or(rows.len().saturating_sub(1)),
        }
    }

    /// The new side's line nearest at or after view line `line`, for a cursor on a removed or
    /// folded line.
    pub fn new_line_near(&self, line: usize) -> Option<usize> {
        match &self.rows {
            None => Some(line),
            Some(rows) => rows[line.min(rows.len().saturating_sub(1))..]
                .iter()
                .find_map(|r| r.new_line())
                .or_else(|| rows.iter().rev().find_map(|r| r.new_line())),
        }
    }

    /// One side of a change, highlighted by `path`'s language: the views' raw material.
    pub fn side(path: &std::path::Path, bytes: &[u8], loader: &Loader) -> File {
        File::new(
            path,
            bytes,
            source::is_binary(bytes),
            vec![],
            vec![],
            loader,
        )
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
        1 + 1 + self.number_width() + 1 + 1
    }

    /// The line-number column: the biggest number (at least 3 wide), or a fold's range.
    pub fn number_width(&self) -> usize {
        let ranges = self
            .rows
            .iter()
            .flatten()
            .filter_map(|r| r.fold_range(self.old_numbers));
        ranges
            .map(|r| r.chars().count())
            .chain([self.numbers.to_string().len(), 3])
            .max()
            .unwrap_or(3)
    }
}

/// `new`, a file read again, with `old`'s view: selections and scroll mapped through the change
/// as Helix maps them when a file changes on disk, and the horizontal scroll and collapse kept.
pub fn keep_view(old: File, mut new: File) -> File {
    if old.text == new.text {
        new.sel = old.sel;
        new.top = old.top;
    } else {
        let tx = glos_helix::text::compare_ropes(&old.text, &new.text);
        let changes = tx.changes();
        new.sel = old.sel.map(changes).ensure_invariants(new.text.slice(..));
        let top = old.text.line_to_char(old.top.min(old.text.len_lines() - 1));
        let top = changes.map_pos(top, glos_helix::text::Assoc::Before);
        new.top = new.text.char_to_line(top.min(new.text.len_chars()));
    }
    new.left = old.left;
    new.collapsed = old.collapsed;
    new
}

/// How `draw` styles a selection: (selection, primary selection, cursor, primary cursor).
pub type SelectionStyles = (Style, Style, Style, Style);

/// (selection, primary selection, cursor, primary cursor) in `mode`, as Helix picks them.
pub fn selection_styles(t: &Theme, mode: &str) -> SelectionStyles {
    let sel = t.get("ui.selection");
    let sel_primary = t.try_get("ui.selection.primary").unwrap_or(sel);
    let base_cursor = t.try_get_exact("ui.cursor").unwrap_or(sel);
    let base_primary = t.try_get("ui.cursor.primary").unwrap_or(base_cursor);
    let cursor = t
        .try_get_exact(&format!("ui.cursor.{mode}"))
        .unwrap_or(base_cursor);
    let primary = t
        .try_get_exact(&format!("ui.cursor.primary.{mode}"))
        .unwrap_or(base_primary);
    (sel, sel_primary, cursor, primary)
}

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
    let computed;
    let spans: &[(usize, Style)] = match (&file.spans, &file.syntax) {
        (Some(s), _) => s,
        (None, Some(s)) => {
            computed = loader.highlights(
                s,
                text,
                text.line_to_byte(top)..text.line_to_byte(last),
                |st: Style, i| st.patch(theme.highlight(i)),
            );
            &computed
        }
        (None, None) => &[],
    };
    let mut span = 0;

    let gutter = if main.is_some() {
        file.gutter_width() as u16
    } else {
        0
    };
    let digits = file.number_width();
    let band = super::diffview::fold_styles(theme).0;
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
        let kind = file.rows.as_ref().and_then(|r| r.get(line)).copied();
        let number = match kind {
            // The empty line after a view's last line has no number.
            None if file.rows.is_some() => None,
            None => Some(line),
            Some(Row::Same { old, new }) => Some(if file.old_numbers { old } else { new }),
            Some(Row::Added(n) | Row::Removed(n) | Row::Line(n)) => Some(n),
            Some(Row::Fold { .. } | Row::Filler) => None,
        };
        let sign = match kind {
            Some(Row::Added(_)) => Some(("▍", added, true)),
            Some(Row::Removed(_)) => Some(("▍", deleted, true)),
            Some(Row::Fold { changes: 1.., .. }) => Some(("▍", modified, true)),
            Some(Row::Line(_)) | None => {
                file.hunks
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
                    })
            }
            Some(_) => None,
        };
        let mark = threads
            .marks
            .iter()
            .find(|(s, e, _)| *s <= line && line <= *e)
            .map(|m| m.2);
        let fold = kind.filter(|k| matches!(k, Row::Fold { .. }));
        let gutter_row = |buf: &mut Buffer, row: usize, y: u16| {
            // A fold is a band across the whole view, gutter and all.
            if fold.is_some() {
                buf.set_style(Rect::new(area.x, y, area.width, 1), band);
            }
            if main.is_none() || gutter == 0 {
                return;
            }
            if let (0, Some(range)) = (row, fold.and_then(|f| f.fold_range(file.old_numbers))) {
                let style = band.patch(theme.get("ui.linenr"));
                buf.set_string(area.x + 2, y, format!("{range:>digits$}"), style);
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
                if let Some(n) = number {
                    buf.set_string(area.x + 2, y, format!("{:>digits$}", n + 1), nr_style);
                }
            }
            if let Some((sign, style, every_row)) = sign
                && (row == 0 || every_row)
            {
                buf.set_string(x0 - 1, y, sign, style);
            }
        };
        gutter_row(buf, 0, y);
        let mut rows = 1;
        let start = text.line_to_char(line);
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
            if matches!(g.raw, Grapheme::Newline) {
                break;
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
        let loader = Loader::new(theme.scopes()).unwrap();
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
        let loader = Loader::new(theme.scopes()).unwrap();
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

#[cfg(test)]
mod cursor_tests {
    use super::*;
    use crate::theme::{ColorSupport, Theme};
    use ratatui::style::Color;

    /// Where the primary cursor is drawn: (column, row) of every cell with its colour.
    fn cursor_cells(text: &str, pos: usize) -> Vec<(u16, u16)> {
        let theme = Theme::load("default", ColorSupport::TrueColor).unwrap();
        let loader = Loader::new(theme.scopes()).unwrap();
        let mut file = File::scratch(text, &loader);
        file.sel = Selection::point(pos);
        let area = Rect::new(0, 0, 10, 4);
        let mut buf = Buffer::empty(area);
        let c = Style::default().bg(Color::Red);
        let styles = (Style::default(), Style::default(), c, c);
        draw(
            &mut buf,
            area,
            &file,
            0,
            0,
            true,
            &loader,
            &theme,
            Some(styles),
            &Threads::default(),
        );
        let mut cells = vec![];
        for y in 0..4 {
            for x in 0..10 {
                if buf[(x, y)].bg == Color::Red {
                    cells.push((x, y));
                }
            }
        }
        cells
    }

    #[test]
    fn one_cursor_at_the_end_of_the_text() {
        assert_eq!(cursor_cells("ab", 2), [(2, 0)], "no trailing newline");
        assert_eq!(cursor_cells("ab\n", 3), [(0, 1)], "on the empty last line");
        assert_eq!(cursor_cells("ab\n\n", 4), [(0, 2)]);
        assert_eq!(cursor_cells("", 0), [(0, 0)]);
        assert_eq!(cursor_cells("ab\n", 2), [(2, 0)], "on the newline");
    }
}
