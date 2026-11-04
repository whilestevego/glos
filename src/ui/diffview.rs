//! The unified and side-by-side views. Each is a read-only document made of lines from the two
//! sides, with a [`Row`] per line saying where it came from, so Helix's motions, search and
//! selections work on it as on any file, and comments map back to the new side.

use glos_helix::text::Rope;
use ratatui::style::Style;

use super::view::{File, Row};
use glos_helix::syntax::Loader;

use crate::{diff::Hunk, theme::Theme};

/// Unchanged lines kept around each change in the unified view, as `diff -u` keeps them.
const CONTEXT: usize = 3;

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
    fold: Style,
}

impl Doc<'_> {
    fn push(&mut self, row: Row) {
        let start = self.text.len();
        let from = match row {
            Row::Same { new, .. } | Row::Added(new) => Some((&self.new, new)),
            Row::Removed(old) => Some((&self.old, old)),
            Row::Fold(_) | Row::Filler => None,
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
            None => {
                let style = if matches!(row, Row::Fold(_)) {
                    self.fold
                } else {
                    Style::default()
                };
                self.spans.push((start, style));
                if let Row::Fold(n) = row {
                    self.text += &format!("⋯ {n} unchanged line{}", if n == 1 { "" } else { "s" });
                }
            }
        }
        self.text.push('\n');
        self.rows.push(row);
    }

    fn file(self, numbers: usize) -> File {
        let tab = self.new.0.tab_width;
        File::synthetic(self.text, tab, self.rows, self.spans, numbers)
    }
}

fn doc<'a>(old: &'a File, new: &'a File, loader: &Loader, theme: &Theme) -> Doc<'a> {
    Doc {
        text: String::new(),
        spans: vec![],
        rows: vec![],
        old: (old, spans_of(old, loader, theme)),
        new: (new, spans_of(new, loader, theme)),
        fold: theme.get("ui.virtual"),
    }
}

/// The classic view: each change's removed lines then its added lines, with three unchanged
/// lines around, and the rest folded into one line.
pub fn unified(old: &File, new: &File, hunks: &[Hunk], loader: &Loader, theme: &Theme) -> File {
    let mut d = doc(old, new, loader, theme);
    let (old_n, new_n) = (line_count(&old.text), line_count(&new.text));
    let (mut o, mut n) = (0, 0);
    // Unchanged lines `len` long from old `o` / new `n`: context after the previous change and
    // before the next, folded between.
    let same = |d: &mut Doc, o: usize, n: usize, len: usize, first: bool, last: bool| {
        let head = if first { 0 } else { CONTEXT.min(len) };
        let tail = if last { 0 } else { CONTEXT.min(len - head) };
        for i in 0..head {
            d.push(Row::Same {
                old: o + i,
                new: n + i,
            });
        }
        if len > head + tail {
            d.push(Row::Fold(len - head - tail));
        }
        for i in len - tail..len {
            d.push(Row::Same {
                old: o + i,
                new: n + i,
            });
        }
    };
    for (i, h) in hunks.iter().enumerate() {
        same(&mut d, o, n, h.after.start as usize - n, i == 0, false);
        for l in h.before.clone() {
            d.push(Row::Removed(l as usize));
        }
        for l in h.after.clone() {
            d.push(Row::Added(l as usize));
        }
        o = h.before.end as usize;
        n = h.after.end as usize;
    }
    if !hunks.is_empty() || new_n > 0 {
        same(&mut d, o, n, new_n - n, hunks.is_empty(), true);
    }
    d.file(old_n.max(new_n))
}

/// Old on the left, new on the right, line for line: the shorter side of each change is padded
/// so both stay level. Both sides are whole.
pub fn split(
    old: &File,
    new: &File,
    hunks: &[Hunk],
    loader: &Loader,
    theme: &Theme,
) -> (File, File) {
    let mut l = doc(old, new, loader, theme);
    let mut r = doc(old, new, loader, theme);
    let (old_n, new_n) = (line_count(&old.text), line_count(&new.text));
    let (mut o, mut n) = (0, 0);
    let same = |l: &mut Doc, r: &mut Doc, len: usize, o: usize, n: usize| {
        for i in 0..len {
            l.push(Row::Same {
                old: o + i,
                new: n + i,
            });
            r.push(Row::Same {
                old: o + i,
                new: n + i,
            });
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
    same(&mut l, &mut r, new_n - n, o, n);
    let mut left = l.file(old_n);
    left.old_numbers = true;
    (left, r.file(new_n))
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

    fn text(f: &File) -> String {
        f.text.to_string()
    }

    fn numbered(n: usize) -> String {
        (1..=n).map(|i| format!("{i}\n")).collect()
    }

    #[test]
    fn unified_keeps_three_lines_around_changes_and_folds_the_rest() {
        let old = numbered(20);
        let new = old.replace("10\n", "ten\n");
        let (o, n, h, l, t) = files(&old, &new);
        let u = unified(&o, &n, &h, &l, &t);
        assert_eq!(
            text(&u),
            "⋯ 6 unchanged lines\n7\n8\n9\n10\nten\n11\n12\n13\n⋯ 7 unchanged lines\n"
        );
        let rows = u.rows.as_ref().unwrap();
        assert_eq!(rows[4], Row::Removed(9));
        assert_eq!(rows[5], Row::Added(9));
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
            "1\none\n2\n3\n4\n5\nfive\n6\n7\n8\n⋯ 2 unchanged lines\n"
        );
        // A new file is all added; a deleted one all removed; no changes, all folded.
        let (o, n, h, l, t) = files("", "a\nb\n");
        assert_eq!(text(&unified(&o, &n, &h, &l, &t)), "a\nb\n");
        let (o, n, h, l, t) = files("a\nb\n", "");
        let u = unified(&o, &n, &h, &l, &t);
        assert_eq!(u.rows.unwrap(), [Row::Removed(0), Row::Removed(1)]);
        let (o, n, h, l, t) = files("a\nb\n", "a\nb\n");
        assert_eq!(text(&unified(&o, &n, &h, &l, &t)), "⋯ 2 unchanged lines\n");
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
