//! Folding in the app: building the view on screen with its folds, the `z` fold keys, and
//! opening folds when something you jump to is inside one. The fold arithmetic is in
//! [`diffview`].

use glos_helix::text::{Range, Selection};

use super::{
    Alt, App, Cur, ViewKind,
    diffview::{self, Built},
    view::{File, Row},
};
use crate::source::Content;

/// The `z` fold keys.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FoldKey {
    /// `zo`, `ret` and a click on a fold line.
    Open,
    /// `za`
    Toggle,
    /// `zR`
    OpenAll,
    /// `zM`
    CloseAll,
    /// `zf`
    Make,
    /// `zd`
    Delete,
    /// `z+` / `z-`: `true` for more.
    Context(bool),
}

impl App {
    /// The threads of the file on screen, as new-side line ranges: they keep their lines in
    /// view.
    fn fold_threads(&self) -> Vec<(usize, usize)> {
        self.fold_threads_in(self.state.main_path(), self.base_file())
    }

    /// The threads of file `path`, whose text is `base`'s, as new-side line ranges.
    fn fold_threads_in(&self, path: &str, base: &File) -> Vec<(usize, usize)> {
        let mut v: Vec<(usize, usize)> = self
            .shown_threads_in(path, base)
            .iter()
            .filter_map(|t| t.anchor.as_ref())
            .filter(|a| a.path == path && a.side == crate::review::Side::New)
            .map(|a| (a.start as usize - 1, a.end as usize - 1))
            .collect();
        v.sort();
        v.dedup();
        v
    }

    fn fold_key(&self) -> (String, ViewKind) {
        (self.state.main_path().to_string(), self.view_kind)
    }

    /// The view of the file on screen, folded as it is now; `None` for a view this file can't
    /// have (unified or side by side of a context file, a binary or collapsed file).
    fn built(&self) -> Option<Built> {
        self.built_for(&self.state.cur, self.state.main_path(), self.base_file())
    }

    /// The view of `cur`, whose path is `path` and whose file is `base`, folded as it is now;
    /// `None` for a binary or collapsed file, or a context file, which has no diff.
    pub(super) fn built_for(&self, cur: &Cur, path: &str, base: &File) -> Option<Built> {
        if base.binary || base.collapsed {
            return None;
        }
        let threads = self.fold_threads_in(path, base);
        let folds = self
            .folds
            .get(&(path.to_string(), self.view_kind))
            .cloned()
            .unwrap_or_default();
        let (kind, ctx) = (self.view_kind, self.diff_context);
        let Cur::Change(i) = *cur else {
            return None; // a context file has no diff
        };
        let change = &self.state.source.changes[i];
        let read = |c: &Content| self.state.source.read(c).ok().flatten().unwrap_or_default();
        let old_bytes = read(&change.old);
        let path = self.state.source.root.join(&change.path);
        let old = File::side(&path, &old_bytes, &self.state.loader);
        // The base file is the new side, already parsed and diffed. A deleted file's base
        // shows its old side instead, so its empty new side is made here.
        let deleted;
        let (new, hunks) = if change.new == Content::None {
            deleted = File::side(&path, &[], &self.state.loader);
            let hunks = crate::diff::hunks(&String::from_utf8_lossy(&old_bytes), "");
            (&deleted, hunks)
        } else {
            (base, base.hunks.clone())
        };
        Some(diffview::build(
            kind,
            &old,
            new,
            &hunks,
            ctx,
            &folds,
            &threads,
            &self.state.loader,
            &self.theme,
        ))
    }

    /// Builds the view for the file on screen, with the cursor on the base file's line and
    /// column. The whole-file view with nothing folded is the file itself: no view is built.
    pub(super) fn build_alt(&mut self) {
        let Some(b) = self.built() else {
            return;
        };
        let threads = self.fold_threads();
        let base = self.base_file();
        let mut doc = b.doc;
        // A view has no notices of its own, and the file's say what kind of change it is.
        doc.notices = base.notices.clone();
        let t = base.text.slice(..);
        let cursor = base.sel.primary().cursor(t);
        let line = t.char_to_line(cursor);
        let col = cursor - t.line_to_char(line);
        let dt = doc.text.slice(..);
        let at = doc.line_of(line).min(dt.len_lines() - 1);
        let len = glos_helix::text::line_end_char_index(&dt, at) - dt.line_to_char(at);
        doc.sel = Selection::point(dt.line_to_char(at) + col.min(len)).ensure_invariants(dt);
        doc.top = at.saturating_sub(glos_helix::commands::SCROLLOFF);
        self.alt = Some(Alt {
            cur: self.state.cur.clone(),
            kind: self.view_kind,
            doc,
            left: b.left,
            full: b.full,
            len: b.len,
            auto: b.auto,
            changed: b.changed,
            lines: b.lines,
            threads,
        });
        self.layout();
        self.scroll_to_cursor(false);
    }

    /// The view on screen, if it's built.
    pub(super) fn shown_alt(&self) -> Option<&Alt> {
        self.alt
            .as_ref()
            .filter(|a| a.cur == self.state.cur && a.kind == self.view_kind)
    }

    /// The row of the whole view on view line `line` (a fold line: its first row).
    fn full_row(&self, line: usize) -> usize {
        match self.shown_alt() {
            Some(a) => a.full.get(line).copied().unwrap_or(a.len),
            None => line,
        }
    }

    /// The view line showing row `row` of the whole view (or the fold holding it).
    fn view_line(&self, row: usize) -> usize {
        match self.shown_alt() {
            Some(a) => a.full.partition_point(|&s| s <= row).saturating_sub(1),
            None => row,
        }
    }

    /// The fold line under the cursor: (first row, rows) of the whole view.
    pub(super) fn fold_here(&self) -> Option<(usize, usize)> {
        let f = self.main_file();
        let t = f.text.slice(..);
        let line = t.char_to_line(f.sel.primary().cursor(t));
        match f.rows.as_ref()?.get(line)? {
            Row::Fold { start, len, .. } => Some((*start, *len)),
            _ => None,
        }
    }

    /// The first new-side line inside the fold under the cursor.
    pub(super) fn fold_first_line(&self) -> Option<usize> {
        let (start, n) = self.fold_here()?;
        let a = self.shown_alt()?;
        a.lines.get(start..start + n)?.iter().find_map(|l| *l)
    }

    /// Builds the view again after its folds changed, keeping the cursor on the same row (or
    /// on row `to`) and the same distance from the top of the screen.
    pub(super) fn refold(&mut self, to: Option<usize>) {
        let f = self.main_file();
        let t = f.text.slice(..);
        let cursor = f.sel.primary().cursor(t);
        let line = t.char_to_line(cursor);
        let col = cursor - t.line_to_char(line);
        let offset = line.saturating_sub(f.top);
        let on_fold = self.fold_here().is_some();
        let row = to.unwrap_or_else(|| self.full_row(line));
        let col = if on_fold || to.is_some() { 0 } else { col };
        if let Some(a) = self.alt.take() {
            self.sync_base(a);
        }
        self.build_alt();
        self.layout();
        let at = self.view_line(row);
        let f = self.main_file_mut();
        let t = f.text.slice(..);
        let at = at.min(t.len_lines() - 1);
        let len = glos_helix::text::line_end_char_index(&t, at) - t.line_to_char(at);
        f.sel = Selection::point(t.line_to_char(at) + col.min(len)).ensure_invariants(t);
        f.top = at.saturating_sub(offset);
        self.scroll_to_cursor(false);
    }

    /// The whole view's row count and what the context folds there, built if need be.
    fn rows_info(&self) -> Option<(usize, Vec<bool>)> {
        match self.shown_alt() {
            Some(a) => Some((a.len, a.auto.clone())),
            None => self.built().map(|b| (b.len, b.auto)),
        }
    }

    pub(super) fn fold_command(&mut self, key: FoldKey) {
        if self.comment_box.is_some() {
            return;
        }
        let Some((len, auto)) = self.rows_info() else {
            self.status = Some(("Nothing to fold here".into(), true));
            return;
        };
        let k = self.fold_key();
        match key {
            FoldKey::Open => match self.fold_here() {
                Some((start, n)) => {
                    self.folds.entry(k).or_default().open(start..start + n, len);
                    self.refold(Some(start));
                }
                None => self.status = Some(("No fold here".into(), false)),
            },
            FoldKey::Toggle => {
                if self.fold_here().is_some() {
                    return self.fold_command(FoldKey::Open);
                }
                let line = self.cursor_line();
                let row = self.full_row(line);
                if self.folds.entry(k).or_default().close_at(row, &auto) {
                    self.refold(Some(row));
                } else {
                    self.status = Some(("No fold here".into(), false));
                }
            }
            FoldKey::OpenAll => {
                let f = self.folds.entry(k).or_default();
                f.opened = vec![true; len];
                for h in &mut f.hand {
                    h.1 = true;
                }
                self.refold(None);
            }
            FoldKey::CloseAll => {
                let f = self.folds.entry(k).or_default();
                f.opened.clear();
                for h in &mut f.hand {
                    h.1 = false;
                }
                self.refold(None);
            }
            FoldKey::Make => {
                let f = self.main_file();
                let t = f.text.slice(..);
                let lines: Vec<(usize, usize)> =
                    f.sel.iter().map(|r: &Range| r.line_range(t)).collect();
                let mut first = usize::MAX;
                let mut made = vec![];
                for (s, e) in lines {
                    let a = self.full_row(s);
                    let b = match f.rows.as_ref().and_then(|r| r.get(e)) {
                        Some(Row::Fold { start, len, .. }) => start + len,
                        _ => self.full_row(e) + 1,
                    };
                    let b = b.min(len);
                    if a < b {
                        made.push((a..b, false));
                        first = first.min(a);
                    }
                }
                if made.is_empty() {
                    self.status = Some(("Nothing to fold here".into(), false));
                    return;
                }
                self.folds.entry(k).or_default().hand.extend(made);
                self.state.mode = super::Mode::Normal;
                self.refold(Some(first));
            }
            FoldKey::Delete => {
                let rows = match self.fold_here() {
                    Some((s, n)) => s..s + n,
                    None => {
                        let r = self.full_row(self.cursor_line());
                        r..r + 1
                    }
                };
                let f = self.folds.entry(k).or_default();
                let before = f.hand.len();
                f.hand
                    .retain(|(h, _)| !(h.start < rows.end && rows.start < h.end));
                if f.hand.len() == before {
                    self.status = Some(("No fold you made here".into(), false));
                } else {
                    self.refold(Some(rows.start));
                }
            }
            FoldKey::Context(more) => {
                self.diff_context = if more {
                    self.diff_context + 1
                } else {
                    self.diff_context.saturating_sub(1)
                };
                let n = self.diff_context;
                self.status = Some((
                    format!("{n} line{} of context", if n == 1 { "" } else { "s" }),
                    false,
                ));
                self.refold(None);
            }
        }
    }

    pub(super) fn cursor_line(&self) -> usize {
        let f = self.main_file();
        let t = f.text.slice(..);
        t.char_to_line(f.sel.primary().cursor(t))
    }

    /// After `]g`, `[g` or opening a file: a cursor on a fold that hides changes opens it, on
    /// its first change.
    pub(super) fn open_fold_on_change(&mut self) {
        let Some((start, n)) = self.fold_here() else {
            return;
        };
        let Some(a) = self.shown_alt() else {
            return;
        };
        let Some(first) = (start..start + n).find(|r| a.changed.get(*r) == Some(&true)) else {
            return;
        };
        let (len, k) = (a.len, self.fold_key());
        self.folds.entry(k).or_default().open(start..start + n, len);
        self.refold(Some(first));
    }

    /// Before going to new-side lines `start..=end` (a thread, a suggestion): folds hiding
    /// them open.
    pub(super) fn reveal(&mut self, start: usize, end: usize) {
        let rows: Vec<usize> = match self.shown_alt() {
            Some(a) => {
                let shown = |l: usize| {
                    a.doc
                        .rows
                        .as_ref()
                        .is_some_and(|r| r.iter().any(|r| r.new_line() == Some(l)))
                };
                if (start..=end).all(shown) {
                    return;
                }
                a.lines
                    .iter()
                    .enumerate()
                    .filter(|(_, l)| l.is_some_and(|l| start <= l && l <= end))
                    .map(|(i, _)| i)
                    .collect()
            }
            None => return,
        };
        let (Some(&a), Some(&b)) = (rows.first(), rows.last()) else {
            return;
        };
        let len = self.shown_alt().map_or(0, |a| a.len);
        let k = self.fold_key();
        self.folds.entry(k).or_default().open(a..b + 1, len);
        self.refold(Some(a));
    }

    /// Threads moved or changed: a view whose kept lines changed is folded again.
    pub(super) fn refold_if_threads_moved(&mut self) {
        if let Some(a) = self.shown_alt()
            && a.threads != self.fold_threads()
        {
            self.refold(None);
        } else if self.shown_alt().is_none() {
            // Nothing is built yet, as when glos starts, or a file may now fold now that a
            // thread moved.
            self.build_alt();
        }
    }

    /// The file's text changed: its folds are worked out afresh.
    pub(super) fn forget_folds(&mut self, path: &str) {
        self.folds.retain(|(p, _), _| p != path);
    }

    /// `mi<ch>` / `ma<ch>` and `]<ch>` / `[<ch>` for Helix's tree-sitter objects, worked out on
    /// the file itself and mapped onto the view, opening the folds they cover: (around or
    /// forward, object letter, goto).
    pub(super) fn ts_object(&mut self, flag: bool, ch: char, goto: bool) {
        let Some(name) = glos_helix::syntax::textobject_name(ch) else {
            return;
        };
        let count = self.count.map_or(1, std::num::NonZeroUsize::get);
        let extend = self.state.mode == super::Mode::Select;
        let base = self.base_file();
        let Some(syntax) = &base.syntax else {
            self.status = Some((
                "Syntax-tree is not available in current buffer".into(),
                true,
            ));
            return;
        };
        let (view, bt) = (self.main_file(), base.text.slice(..));
        let primary = view.sel.primary_index();
        // Each selection on the file, and the object found from it.
        let found: Vec<(Range, Option<Range>)> = view
            .sel
            .iter()
            .map(|r| {
                let br = Range::new(to_base(view, base, r.anchor), to_base(view, base, r.head));
                let got = if goto {
                    self.state
                        .loader
                        .goto_object(syntax, bt, br, name, flag, count)
                } else {
                    self.state.loader.textobject(syntax, bt, br, name, flag)
                };
                (br, (got != br).then_some(got))
            })
            .collect();
        // The objects' lines come out of any fold, so the selection is all of them.
        let lines: Vec<(usize, usize)> = found
            .iter()
            .filter_map(|(_, g)| *g)
            .map(|g| {
                let s = bt.char_to_line(g.from());
                (s, bt.char_to_line(g.to().saturating_sub(1)).max(s))
            })
            .collect();
        for (s, e) in lines {
            self.reveal(s, e);
        }
        let (view, base) = (self.main_file(), self.base_file());
        let ranges: Vec<Range> = found
            .into_iter()
            .map(|(br, got)| {
                let r = Range::new(to_view(view, base, br.anchor), to_view(view, base, br.head));
                let Some(got) = got else {
                    return r;
                };
                let (a, h) = (
                    to_view(view, base, got.anchor),
                    to_view(view, base, got.head),
                );
                if goto && extend {
                    // Select mode keeps the anchor, as Helix's goto_ts_object_impl.
                    let head = if h < r.anchor { a } else { h };
                    Range::new(r.anchor, head)
                } else if goto {
                    let nr = Range::new(a, h);
                    if flag { nr } else { nr.flip() }
                } else {
                    Range::new(a, h).with_direction(br.direction())
                }
            })
            .collect();
        let f = self.main_file_mut();
        f.sel = Selection::new(ranges.into(), primary).ensure_invariants(f.text.slice(..));
        self.scroll_to_cursor(false);
    }
}

/// A position in the view on screen, on the file: on a new-side line, the same column; on a
/// removed or fold line, the start of the next new-side line.
fn to_base(view: &File, base: &File, pos: usize) -> usize {
    if view.rows.is_none() {
        return pos;
    }
    let (vt, bt) = (view.text.slice(..), base.text.slice(..));
    let line = vt.char_to_line(pos);
    let col = pos - vt.line_to_char(line);
    // Past the view's last row is the end of the file.
    let past_end = view.rows.as_ref().is_some_and(|r| line >= r.len());
    let (false, Some(n)) = (past_end, view.new_line_near(line)) else {
        return bt.len_chars();
    };
    let n = n.min(bt.len_lines() - 1);
    let len = glos_helix::text::line_end_char_index(&bt, n) - bt.line_to_char(n);
    let col = if view.new_line_at(line).is_some() {
        col.min(len)
    } else {
        0
    };
    bt.line_to_char(n) + col
}

/// A position in the file, in the view on screen: its line's row (or the next shown).
fn to_view(view: &File, base: &File, pos: usize) -> usize {
    if view.rows.is_none() {
        return pos;
    }
    let (vt, bt) = (view.text.slice(..), base.text.slice(..));
    let line = bt.char_to_line(pos.min(bt.len_chars()));
    let col = pos - bt.line_to_char(line);
    let at = view.line_of(line).min(vt.len_lines() - 1);
    let len = glos_helix::text::line_end_char_index(&vt, at) - vt.line_to_char(at);
    let col = if view.new_line_at(at) == Some(line) {
        col.min(len)
    } else {
        0
    };
    vt.line_to_char(at) + col
}
