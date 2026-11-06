//! The views as documents: the whole file, unified, and side by side. Each is read-only text
//! made of lines from the two sides, with a [`Row`] per line saying where it came from, so
//! Helix's motions, search and selections work on it as on any file, and comments map back to
//! the new side.
//!
//! A view is built in two steps: every row of it ([`rows`]), then the fold, which hides
//! unchanged rows away from changes and threads, and the folds you made ([`Folds`]).

use std::{collections::HashMap, ops::Range};

use glos_helix::text::Rope;
use ratatui::style::Style;

use super::{
    ViewKind,
    view::{File, Row},
};
use glos_helix::syntax::Loader;

use crate::{diff::Hunk, theme::Theme};

/// A file's lines as the diff counts them: a final newline doesn't start another line.
fn line_count(text: &Rope) -> usize {
    let n = text.len_lines();
    if n > 1 && text.line(n - 1).len_chars() == 0 {
        n - 1
    } else if text.len_chars() == 0 {
        0
    } else {
        n
    }
}

fn line_text(f: &File, n: usize) -> String {
    let line = f.text.line(n).to_string();
    line.trim_end_matches(['\n', '\r']).to_string()
}

/// The whole file's highlighting, as (start byte, style) spans.
fn spans_of(f: &File, loader: &Loader, theme: &Theme) -> Vec<(usize, Style)> {
    match &f.syntax {
        Some(s) => {
            let text = f.text.slice(..);
            loader.highlights(s, text, 0..text.len_bytes(), |st: Style, i| {
                st.patch(theme.highlight(i))
            })
        }
        None => vec![],
    }
}

/// Builds a view's text and spans row by row.
struct Doc<'a> {
    text: String,
    spans: Vec<(usize, Style)>,
    rows: Vec<Row>,
    old: (&'a File, Vec<(usize, Style)>),
    new: (&'a File, Vec<(usize, Style)>),
    /// The fold band, and its key hint.
    fold: (Style, Style),
    /// The old side of side by side: its fold lines give old line numbers.
    old_side: bool,
    /// The changed words of each side's lines, and of the view's lines as they're pushed.
    words: (&'a Words, &'a Words),
    view_words: Words,
}

/// Changed words by line, as byte ranges within the line (see `File::words`).
type Words = HashMap<usize, Vec<Range<usize>>>;

impl Doc<'_> {
    fn push(&mut self, row: Row) {
        let start = self.text.len();
        let words = match row {
            Row::Added(n) | Row::Line(n) => self.words.1.get(&n),
            Row::Removed(o) => self.words.0.get(&o),
            _ => None,
        };
        if let Some(w) = words {
            self.view_words.insert(self.rows.len(), w.clone());
        }
        let from = match row {
            Row::Same { new, .. } | Row::Added(new) | Row::Line(new) => Some((&self.new, new)),
            Row::Removed(old) => Some((&self.old, old)),
            Row::Fold { .. } | Row::Filler => None,
        };
        match from {
            Some(((file, spans), line)) => {
                let text = line_text(file, line);
                // The source line's spans, moved to where the line now starts.
                let lb = file.text.line_to_byte(line);
                let le = lb + text.len();
                let i = spans.partition_point(|(b, _)| *b <= lb);
                let first = if i > 0 {
                    spans[i - 1].1
                } else {
                    Style::default()
                };
                self.spans.push((start, first));
                for (b, st) in spans[i..].iter().take_while(|(b, _)| *b < le) {
                    self.spans.push((start + b - lb, *st));
                }
                self.text += &text;
            }
            None => match row {
                Row::Fold {
                    len,
                    changes,
                    threads,
                    ..
                } => {
                    let range = row.fold_range(self.old_side);
                    let (text, hint) = fold_text(len, changes, threads, range.as_deref());
                    self.spans.push((start, self.fold.0));
                    self.text += &text;
                    self.spans.push((self.text.len(), self.fold.1));
                    self.text += &hint;
                }
                _ => self.spans.push((start, Style::default())),
            },
        }
        self.text.push('\n');
        self.rows.push(row);
    }

    fn file(self, numbers: usize) -> File {
        let tab = self.new.0.tab_width;
        let mut f = File::synthetic(self.text, tab, self.rows, self.spans, numbers);
        f.words = self.view_words;
        f
    }
}

fn doc<'a>(
    old: &'a File,
    new: &'a File,
    words: (&'a Words, &'a Words),
    loader: &Loader,
    theme: &Theme,
) -> Doc<'a> {
    Doc {
        words,
        view_words: Words::new(),
        text: String::new(),
        spans: vec![],
        rows: vec![],
        // The whole-file view has one side.
        old: (
            old,
            if std::ptr::eq(old, new) {
                vec![]
            } else {
                spans_of(old, loader, theme)
            },
        ),
        new: (new, spans_of(new, loader, theme)),
        fold: fold_styles(theme),
        old_side: false,
    }
}

/// The fold band: the theme's `ui.virtual.fold` if it has one, else text on the current line's
/// background (`ui.cursorline.primary`, or the popups'), as a quiet band across the view. Its
/// key hint is in `ui.virtual`'s dimmer colour.
pub fn fold_styles(theme: &Theme) -> (Style, Style) {
    let band = theme.try_get_exact("ui.virtual.fold").unwrap_or_else(|| {
        let bg = [
            "ui.cursorline.primary",
            "ui.cursorline",
            "ui.popup",
            "ui.statusline",
        ]
        .iter()
        .find_map(|k| theme.try_get(k).and_then(|s| s.bg));
        let mut s = theme.get("ui.text");
        s.bg = bg.or(s.bg);
        s
    });
    let mut hint = band;
    hint.fg = theme.get("ui.virtual").fg.or(band.fg);
    (band, hint)
}

fn plural(n: usize, one: &str) -> String {
    format!("{n} {one}{}", if n == 1 { "" } else { "s" })
}

/// A fold line, and its key hint: `↕ lines 16–31 (16)`, or for a fold you made over changes
/// or threads, `↕ lines 16–31 (16) · 3 changed · 1 thread`; then `  ret opens`.
fn fold_text(len: usize, changes: usize, threads: usize, range: Option<&str>) -> (String, String) {
    let mut s = match range {
        Some(r) if r.contains('–') => format!("↕ lines {r} ({len})"),
        Some(r) => format!("↕ line {r} ({len})"),
        // Only removed lines, in a fold you made in the unified view.
        None => format!("↕ {}", plural(len, "row")),
    };
    if changes > 0 {
        s += &format!(" · {changes} changed");
    }
    if threads > 0 {
        s += &format!(" · {}", plural(threads, "thread"));
    }
    (s, "  ret opens".into())
}

/// What's folded in one view of one file, in rows of the whole view (before folding). It's
/// kept while you switch files, and cleared when the file changes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Folds {
    /// Rows you opened (`zo`, `zR`, `ret`): shown even where the context would fold them.
    pub opened: Vec<bool>,
    /// Folds you made (`zf`), and whether each is open.
    pub hand: Vec<(std::ops::Range<usize>, bool)>,
}

impl Folds {
    fn opened(&self, row: usize) -> bool {
        self.opened.get(row).copied().unwrap_or(false)
    }

    pub fn open(&mut self, rows: std::ops::Range<usize>, len: usize) {
        self.opened.resize(len, false);
        for r in rows.clone() {
            if let Some(o) = self.opened.get_mut(r) {
                *o = true;
            }
        }
        for (h, open) in &mut self.hand {
            if h.start < rows.end && rows.start < h.end {
                *open = true;
            }
        }
    }

    /// `za` on a shown row: closes the innermost open fold you made around it, or else the
    /// opened stretch the context folds it in. Returns whether anything closed.
    pub fn close_at(&mut self, row: usize, auto: &[bool]) -> bool {
        if let Some((_, open)) = self
            .hand
            .iter_mut()
            .filter(|(h, open)| *open && h.contains(&row))
            .min_by_key(|(h, _)| h.len())
        {
            *open = false;
            return true;
        }
        if !auto.get(row).copied().unwrap_or(false) {
            return false;
        }
        let (mut a, mut b) = (row, row + 1);
        while a > 0 && auto[a - 1] {
            a -= 1;
        }
        while b < auto.len() && auto[b] {
            b += 1;
        }
        let mut closed = false;
        for o in self.opened.iter_mut().take(b).skip(a) {
            closed |= *o;
            *o = false;
        }
        closed
    }
}

/// Unchanged rows the context folds: those more than `context` rows from any row in `keep` (a
/// change or a thread), in stretches of at least two. Nothing, if nothing is kept.
pub fn auto_folded(keep: &[bool], context: usize) -> Vec<bool> {
    let n = keep.len();
    if !keep.contains(&true) {
        return vec![false; n];
    }
    // Distance to the nearest kept row, from either side.
    let mut dist = vec![usize::MAX; n];
    let mut last = None;
    for i in 0..n {
        if keep[i] {
            last = Some(i);
        }
        if let Some(l) = last {
            dist[i] = i - l;
        }
    }
    last = None;
    for i in (0..n).rev() {
        if keep[i] {
            last = Some(i);
        }
        if let Some(l) = last {
            dist[i] = dist[i].min(l - i);
        }
    }
    let mut out: Vec<bool> = dist.iter().map(|d| *d > context).collect();
    drop_short(&mut out);
    out
}

/// Folding one row would only swap it for a fold row.
fn drop_short(hidden: &mut [bool]) {
    let n = hidden.len();
    for i in 0..n {
        if hidden[i] && (i == 0 || !hidden[i - 1]) && (i + 1 == n || !hidden[i + 1]) {
            hidden[i] = false;
        }
    }
}

/// The rows hidden: what the context folds, less what you opened, and every closed fold you
/// made (whatever it holds).
pub fn hidden(keep: &[bool], context: usize, folds: &Folds) -> Vec<bool> {
    let mut h: Vec<bool> = auto_folded(keep, context)
        .iter()
        .enumerate()
        .map(|(i, a)| *a && !folds.opened(i))
        .collect();
    drop_short(&mut h);
    for (r, open) in &folds.hand {
        if !open {
            for i in r.clone() {
                if let Some(x) = h.get_mut(i) {
                    *x = true;
                }
            }
        }
    }
    h
}

/// A view's rows before folding: the new side's rows (both sides' for side by side), and which
/// rows are changes.
pub struct Rows {
    pub rows: Vec<Row>,
    /// The old side, for side by side: level with `rows`.
    pub left: Option<Vec<Row>>,
    pub changed: Vec<bool>,
}

/// Every row of a view of `new` (and `old`), unfolded.
pub fn rows(kind: ViewKind, old: &File, new: &File, hunks: &[Hunk]) -> Rows {
    let (old_n, new_n) = (line_count(&old.text), line_count(&new.text));
    match kind {
        ViewKind::File => {
            let mut changed = vec![false; new_n];
            for h in hunks {
                let (s, e) = (h.after.start as usize, h.after.end as usize);
                // A removal marks the line after it, as the gutter does.
                for c in changed.iter_mut().take(e.max(s + 1)).skip(s) {
                    *c = true;
                }
                if s >= new_n && new_n > 0 {
                    changed[new_n - 1] = true;
                }
            }
            Rows {
                rows: (0..new_n).map(Row::Line).collect(),
                left: None,
                changed,
            }
        }
        ViewKind::Unified => {
            let mut rows = vec![];
            let (mut o, mut n) = (0, 0);
            let same = |rows: &mut Vec<Row>, o: usize, n: usize, len: usize| {
                rows.extend((0..len).map(|i| Row::Same {
                    old: o + i,
                    new: n + i,
                }))
            };
            for h in hunks {
                same(&mut rows, o, n, h.after.start as usize - n);
                rows.extend(h.before.clone().map(|l| Row::Removed(l as usize)));
                rows.extend(h.after.clone().map(|l| Row::Added(l as usize)));
                o = h.before.end as usize;
                n = h.after.end as usize;
            }
            same(&mut rows, o, n, new_n.saturating_sub(n));
            let changed = rows
                .iter()
                .map(|r| matches!(r, Row::Added(_) | Row::Removed(_)))
                .collect();
            let _ = old_n;
            Rows {
                rows,
                left: None,
                changed,
            }
        }
        ViewKind::Split => {
            let (mut l, mut r) = (vec![], vec![]);
            let (mut o, mut n) = (0, 0);
            let same = |l: &mut Vec<Row>, r: &mut Vec<Row>, len: usize, o: usize, n: usize| {
                for i in 0..len {
                    let row = Row::Same {
                        old: o + i,
                        new: n + i,
                    };
                    l.push(row);
                    r.push(row);
                }
            };
            for h in hunks {
                same(&mut l, &mut r, h.after.start as usize - n, o, n);
                let (bl, al) = (h.before.len(), h.after.len());
                for i in 0..bl.max(al) {
                    l.push(if i < bl {
                        Row::Removed(h.before.start as usize + i)
                    } else {
                        Row::Filler
                    });
                    r.push(if i < al {
                        Row::Added(h.after.start as usize + i)
                    } else {
                        Row::Filler
                    });
                }
                o = h.before.end as usize;
                n = h.after.end as usize;
            }
            same(&mut l, &mut r, new_n.saturating_sub(n), o, n);
            let changed = l
                .iter()
                .zip(&r)
                .map(|(a, b)| !matches!((a, b), (Row::Same { .. }, Row::Same { .. })))
                .collect();
            Rows {
                rows: r,
                left: Some(l),
                changed,
            }
        }
    }
}

/// A view, folded: its document (the new side's, for side by side), the old side's for side by
/// side, and for each of its lines the row of the whole view it shows (a fold line, its first).
pub struct Built {
    pub doc: File,
    pub left: Option<File>,
    pub full: Vec<usize>,
    /// The whole view's row count, to size [`Folds::opened`].
    pub len: usize,
    /// What the context folds, before what you opened: for `za`.
    pub auto: Vec<bool>,
    /// The whole view's changed rows, and each row's new-side line.
    pub changed: Vec<bool>,
    pub lines: Vec<Option<usize>>,
    /// Whether anything is folded.
    pub folded: bool,
}

/// Builds a view of `new` (and `old`), folded by `context`, `folds`, and the threads (as
/// new-side line ranges, inclusive) that keep their lines in view.
#[allow(clippy::too_many_arguments)]
pub fn build(
    kind: ViewKind,
    old: &File,
    new: &File,
    hunks: &[Hunk],
    context: usize,
    folds: &Folds,
    threads: &[(usize, usize)],
    loader: &Loader,
    theme: &Theme,
) -> Built {
    let r = rows(kind, old, new, hunks);
    let n = r.rows.len();
    // A thread keeps its lines, and is counted on the row of its last line.
    let mut keep = r.changed.clone();
    let mut thread_rows = vec![];
    for (s, e) in threads {
        for (i, row) in r.rows.iter().enumerate() {
            if let Some(l) = row.new_line()
                && *s <= l
                && l <= *e
            {
                keep[i] = true;
                if l == *e {
                    thread_rows.push(i);
                }
            }
        }
    }
    let auto = auto_folded(&keep, context);
    let hidden = hidden(&keep, context, folds);
    let mut mine = vec![false; n];
    for (r, open) in &folds.hand {
        if !open {
            for m in mine.iter_mut().take(r.end).skip(r.start) {
                *m = true;
            }
        }
    }
    let folded = hidden.contains(&true);
    if kind == ViewKind::File && !folded {
        // Not shown: the whole-file view with nothing folded is the file itself.
        return Built {
            doc: File::synthetic(String::new(), new.tab_width, vec![], vec![], 0),
            left: None,
            full: vec![],
            len: n,
            auto,
            lines: r.rows.iter().map(|r| r.new_line()).collect(),
            changed: r.changed,
            folded,
        };
    }
    // The whole-file view has its words already. The other views work them out from the two
    // sides, as they do the hunks.
    let computed;
    let words = if std::ptr::eq(old, new) {
        (&old.words, &new.words)
    } else {
        let w = crate::diff_words::changed(&old.text.to_string(), &new.text.to_string(), hunks);
        computed = (w.old, w.new);
        (&computed.0, &computed.1)
    };
    let mut d = doc(old, new, words, loader, theme);
    let mut ld = r.left.as_ref().map(|_| Doc {
        old_side: true,
        ..doc(old, new, words, loader, theme)
    });
    let mut full = vec![];
    let mut i = 0;
    while i < n {
        if !hidden[i] {
            d.push(r.rows[i]);
            if let (Some(ld), Some(l)) = (&mut ld, &r.left) {
                ld.push(l[i]);
            }
            full.push(i);
            i += 1;
            continue;
        }
        // A fold you made is a line of its own, apart from what the context folds next to it.
        let start = i;
        while i < n && hidden[i] && mine[i] == mine[start] {
            i += 1;
        }
        // The lines hidden on each side: the old side's from its own rows in side by side.
        let span = |lines: Vec<usize>| Some((*lines.first()?, *lines.last()?));
        let olds = r.left.as_ref().unwrap_or(&r.rows)[start..i].iter();
        let fold = Row::Fold {
            start,
            len: i - start,
            changes: r.changed[start..i].iter().filter(|c| **c).count(),
            threads: thread_rows
                .iter()
                .filter(|t| (start..i).contains(*t))
                .count(),
            lines: span(
                r.rows[start..i]
                    .iter()
                    .filter_map(|r| r.new_line())
                    .collect(),
            ),
            old_lines: span(olds.filter_map(|r| r.old_line()).collect()),
        };
        d.push(fold);
        if let Some(ld) = &mut ld {
            ld.push(fold);
        }
        full.push(start);
    }
    let (old_n, new_n) = (line_count(&old.text), line_count(&new.text));
    let (mut doc, left) = match kind {
        ViewKind::File => {
            let mut f = d.file(new.numbers);
            // The file's own changes, onto the view's lines, so the gutter and `]g` see them.
            let view_of = |row: usize| full.partition_point(|&s| s <= row).saturating_sub(1);
            f.hunks = hunks
                .iter()
                .map(|h| {
                    let s = h.after.start as usize;
                    let start = if s >= n { full.len() } else { view_of(s) } as u32;
                    let end = if h.after.is_empty() {
                        start
                    } else {
                        view_of(h.after.end as usize - 1) as u32 + 1
                    };
                    Hunk {
                        before: h.before.clone(),
                        after: start..end,
                    }
                })
                .collect();
            (f, None)
        }
        ViewKind::Unified => (d.file(old_n.max(new_n)), None),
        ViewKind::Split => {
            let mut left = ld.unwrap().file(old_n);
            left.old_numbers = true;
            (d.file(new_n), Some(left))
        }
    };
    doc.tab_width = new.tab_width;
    Built {
        doc,
        left,
        full,
        len: n,
        auto,
        lines: r.rows.iter().map(|r| r.new_line()).collect(),
        changed: r.changed,
        folded,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{diff, theme::ColorSupport};

    fn files(old: &str, new: &str) -> (File, File, Vec<Hunk>, Loader, Theme) {
        let theme = Theme::load("default", ColorSupport::TrueColor).unwrap();
        let loader = Loader::new(theme.scopes()).unwrap();
        let p = std::path::Path::new("x.rs");
        let (o, n) = (
            File::side(p, old.as_bytes(), &loader),
            File::side(p, new.as_bytes(), &loader),
        );
        (o, n, diff::hunks(old, new), loader, theme)
    }

    fn unified(o: &File, n: &File, h: &[Hunk], l: &Loader, t: &Theme) -> File {
        build(ViewKind::Unified, o, n, h, 3, &Folds::default(), &[], l, t).doc
    }

    fn split(o: &File, n: &File, h: &[Hunk], l: &Loader, t: &Theme) -> (File, File) {
        let b = build(ViewKind::Split, o, n, h, 3, &Folds::default(), &[], l, t);
        (b.left.unwrap(), b.doc)
    }

    fn text(f: &File) -> String {
        f.text.to_string()
    }

    fn numbered(n: usize) -> String {
        (1..=n).map(|i| format!("{i}\n")).collect()
    }

    /// Which rows `keep` and `context` fold, as a string: `.` shown, `#` folded.
    fn folded(keep: &str, context: usize) -> String {
        let keep: Vec<bool> = keep.chars().map(|c| c == 'x').collect();
        auto_folded(&keep, context)
            .iter()
            .map(|h| if *h { '#' } else { '.' })
            .collect()
    }

    #[test]
    fn the_context_folds_what_is_far_from_changes() {
        // x is a change: `context` rows each side stay.
        assert_eq!(folded("....x.....", 1), "###...####");
        assert_eq!(folded("....x.....", 0), "####.#####");
        // Only row 0 is more than 3 away, and one row alone isn't folded.
        assert_eq!(folded("....x.....", 3), "........##");
        // Changes at the very start and end.
        assert_eq!(folded("x........x", 2), "...####...");
        // Close changes: their context meets, so nothing between folds.
        assert_eq!(folded("..x...x..", 1), ".........");
        assert_eq!(folded("x...x", 1), ".....");
        // Nothing changed, nothing folded.
        assert_eq!(folded(".....", 0), ".....");
    }

    #[test]
    fn opened_rows_and_your_folds() {
        let keep: Vec<bool> = "x.........".chars().map(|c| c == 'x').collect();
        let show = |f: &Folds| -> String {
            hidden(&keep, 1, f)
                .iter()
                .map(|h| if *h { '#' } else { '.' })
                .collect()
        };
        let mut f = Folds::default();
        assert_eq!(show(&f), "..########");
        // Opening rows 4..7 leaves two folds, the short one shown.
        f.open(4..7, 10);
        assert_eq!(show(&f), "..##...###");
        f.open(2..3, 10);
        assert_eq!(show(&f), ".......###", "a single folded row shows");
        // Your fold hides even the change.
        f.hand.push((0..2, false));
        assert_eq!(show(&f), "##.....###");
        // `za` on the open rows closes the stretch the context folds them in.
        let auto = auto_folded(&keep, 1);
        assert!(f.close_at(5, &auto));
        assert_eq!(show(&f), "##########");
        // On your fold, `za` closes it first; the change row isn't folded by the context.
        f.hand[0].1 = true;
        assert!(f.close_at(0, &auto));
        assert!(!f.hand[0].1);
        assert!(!f.close_at(0, &auto), "nothing open there any more");
        // Opening across your fold opens it too.
        f.open(0..1, 10);
        assert!(f.hand[0].1);
    }

    #[test]
    fn unified_keeps_three_lines_around_changes_and_folds_the_rest() {
        let old = numbered(20);
        let new = old.replace("10\n", "ten\n");
        let (o, n, h, l, t) = files(&old, &new);
        let u = unified(&o, &n, &h, &l, &t);
        assert_eq!(
            text(&u),
            "↕ lines 1–6 (6)  ret opens\n7\n8\n9\n10\nten\n11\n12\n13\n↕ lines 14–20 (7)  ret opens\n"
        );
        let rows = u.rows.as_ref().unwrap();
        assert_eq!(rows[4], Row::Removed(9));
        assert_eq!(rows[5], Row::Added(9));
        assert_eq!(
            rows[0],
            Row::Fold {
                start: 0,
                len: 6,
                changes: 0,
                threads: 0,
                lines: Some((0, 5)),
                old_lines: Some((0, 5)),
            }
        );
        // New-side lines map both ways; removed and folded lines have none.
        assert_eq!(u.new_line_at(5), Some(9));
        assert_eq!(u.new_line_at(4), None);
        assert_eq!(u.line_of(9), 5);
        assert_eq!(u.line_of(2), 1, "a folded line maps to the next shown");
        assert_eq!(u.new_line_near(0), Some(6));
        assert_eq!(u.numbers, 20);
    }

    #[test]
    fn unified_joins_close_changes_and_handles_the_ends() {
        let old = numbered(10);
        // Changes at lines 1 and 5: their context overlaps, so there's no fold between.
        let new = old.replace("1\n", "one\n").replacen("5\n", "five\n", 1);
        let (o, n, h, l, t) = files(&old, &new);
        let u = unified(&o, &n, &h, &l, &t);
        assert_eq!(
            text(&u),
            "1\none\n2\n3\n4\n5\nfive\n6\n7\n8\n↕ lines 9–10 (2)  ret opens\n"
        );
        // A new file is all added; a deleted one all removed; with no changes, nothing folds.
        let (o, n, h, l, t) = files("", "a\nb\n");
        assert_eq!(text(&unified(&o, &n, &h, &l, &t)), "a\nb\n");
        let (o, n, h, l, t) = files("a\nb\n", "");
        let u = unified(&o, &n, &h, &l, &t);
        assert_eq!(u.rows.unwrap(), [Row::Removed(0), Row::Removed(1)]);
        let (o, n, h, l, t) = files("a\nb\n", "a\nb\n");
        assert_eq!(text(&unified(&o, &n, &h, &l, &t)), "a\nb\n");
    }

    #[test]
    fn context_setting_and_threads_keep_lines() {
        let old = numbered(30);
        let new = old.replace("10\n", "ten\n");
        let (o, n, h, l, t) = files(&old, &new);
        let b = |ctx: usize, threads: &[(usize, usize)]| {
            text(
                &build(
                    ViewKind::Unified,
                    &o,
                    &n,
                    &h,
                    ctx,
                    &Folds::default(),
                    threads,
                    &l,
                    &t,
                )
                .doc,
            )
        };
        assert_eq!(
            b(0, &[]),
            "↕ lines 1–9 (9)  ret opens\n10\nten\n↕ lines 11–30 (20)  ret opens\n"
        );
        assert_eq!(
            b(1, &[]),
            "↕ lines 1–8 (8)  ret opens\n9\n10\nten\n11\n↕ lines 12–30 (19)  ret opens\n"
        );
        // A thread on line 25 (0-based 24) keeps it and its context.
        assert_eq!(
            b(0, &[(24, 24)]),
            "↕ lines 1–9 (9)  ret opens\n10\nten\n↕ lines 11–24 (14)  ret opens\n25\n↕ lines 26–30 (5)  ret opens\n"
        );
    }

    #[test]
    fn the_file_view_folds_with_the_gutter_on_its_lines() {
        let old = numbered(30);
        let new = old.replace("10\n", "ten\n").replace("20\n", "");
        let (_, n, _, l, t) = files(&old, &new);
        let mut n = n;
        n.hunks = diff::hunks(&old, &new);
        let b = build(
            ViewKind::File,
            &n,
            &n,
            &n.hunks,
            1,
            &Folds::default(),
            &[],
            &l,
            &t,
        );
        assert!(b.folded);
        assert_eq!(
            text(&b.doc),
            "↕ lines 1–8 (8)  ret opens\n9\nten\n11\n↕ lines 12–18 (7)  ret opens\n19\n21\n22\n↕ lines 22–29 (8)  ret opens\n"
        );
        let rows = b.doc.rows.as_ref().unwrap();
        assert_eq!(rows[2], Row::Line(9));
        assert_eq!(b.doc.numbers, n.numbers);
        // The changes, on the view's lines: `ten` replaced, and line 20 removed before `21`.
        let after: Vec<_> = b.doc.hunks.iter().map(|h| h.after.clone()).collect();
        assert_eq!(after, [2..3, 6..6]);
        assert_eq!(b.full, [0, 8, 9, 10, 11, 18, 19, 20, 21]);
        // Everything opened: the file isn't built at all.
        let mut f = Folds::default();
        f.open(0..b.len, b.len);
        let b = build(ViewKind::File, &n, &n, &n.hunks, 1, &f, &[], &l, &t);
        assert!(!b.folded);
    }

    #[test]
    fn a_fold_you_made_says_what_it_hides() {
        let old = numbered(10);
        let new = old.replace("5\n", "five\n");
        let (o, n, h, l, t) = files(&old, &new);
        let mut f = Folds::default();
        // Rows 2..9 of the unified view: 3, 4, the removed 5, the added five, 6, 7 and 8.
        f.hand.push((2..9, false));
        let b = build(ViewKind::Unified, &o, &n, &h, 3, &f, &[(5, 5)], &l, &t);
        assert_eq!(
            text(&b.doc),
            "1\n2\n↕ lines 3–8 (7) · 2 changed · 1 thread  ret opens\n9\n10\n"
        );
        // Next to what the context folds, it stays a line of its own.
        let mut f = Folds::default();
        f.hand.push((0..2, false));
        let old = numbered(20);
        let (o2, n2, h2, _, _) = files(&old, &old.replace("10\n", "ten\n"));
        let b2 = build(ViewKind::Unified, &o2, &n2, &h2, 3, &f, &[], &l, &t);
        assert!(
            text(&b2.doc)
                .starts_with("↕ lines 1–2 (2)  ret opens\n↕ lines 3–6 (4)  ret opens\n7\n")
        );
        // `]g` still finds the hidden change: the fold line is one.
        assert_eq!(b.doc.hunks.len(), 1);
        assert_eq!(b.doc.hunks[0].after, 2..3);
        assert_eq!(fold_text(1, 0, 0, Some("5")).0, "↕ line 5 (1)");
        assert_eq!(
            fold_text(3, 1, 2, Some("2–4")).0,
            "↕ lines 2–4 (3) · 1 changed · 2 threads"
        );
        assert_eq!(fold_text(3, 0, 0, None).0, "↕ 3 rows", "only removed lines");
    }

    #[test]
    fn split_keeps_both_sides_level() {
        let (o, n, h, l, t) = files("a\nb\nc\nd\n", "a\nB1\nB2\nB3\nd\ne\n");
        let (left, right) = split(&o, &n, &h, &l, &t);
        assert_eq!(text(&left), "a\nb\nc\n\nd\n\n");
        assert_eq!(text(&right), "a\nB1\nB2\nB3\nd\ne\n");
        assert_eq!(left.text.len_lines(), right.text.len_lines());
        assert_eq!(left.rows.as_ref().unwrap()[3], Row::Filler);
        assert!(left.old_numbers && !right.old_numbers);
        assert_eq!(right.line_of(4), 4);
    }

    #[test]
    fn split_folds_both_sides_alike() {
        let old = numbered(20);
        let new = old.replace("10\n", "ten\n");
        let (o, n, h, l, t) = files(&old, &new);
        let (left, right) = split(&o, &n, &h, &l, &t);
        assert_eq!(left.text.len_lines(), right.text.len_lines());
        assert_eq!(
            text(&right),
            "↕ lines 1–6 (6)  ret opens\n7\n8\n9\nten\n11\n12\n13\n↕ lines 14–20 (7)  ret opens\n"
        );
        assert!(text(&left).starts_with("↕ lines 1–6 (6)  ret opens\n7\n8\n9\n10\n"));
    }

    #[test]
    fn highlighting_follows_lines_into_the_view() {
        let old = "fn a() {}\n".repeat(10);
        let new = old.replacen("fn a() {}\n", "let x = 1;\n", 1);
        let (o, n, h, l, t) = files(&old, &new);
        let u = unified(&o, &n, &h, &l, &t);
        let spans = u.spans.as_ref().unwrap();
        // Spans start in order and inside the text, and `fn` keeps the keyword style.
        assert!(spans.windows(2).all(|w| w[0].0 <= w[1].0));
        assert!(spans.iter().all(|(b, _)| *b <= u.text.len_bytes()));
        let style_at = |b: usize| spans.iter().rev().find(|(s, _)| *s <= b).unwrap().1;
        let fn_at = u.text.to_string().find("fn").unwrap();
        assert_ne!(style_at(fn_at), Style::default(), "highlighted");
        // The same keyword in the source file has the same style.
        let src = spans_of(&n, &l, &t);
        let src_fn = n.text.to_string().find("fn").unwrap();
        let src_style = src.iter().rev().find(|(s, _)| *s <= src_fn).unwrap().1;
        assert_eq!(style_at(fn_at), src_style);
    }
}
