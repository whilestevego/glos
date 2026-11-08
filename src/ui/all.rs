//! All-files mode: every changed file in one scroll, each under a header with its path, as
//! GitHub's "Files changed" page shows a pull request.
//!
//! The cursor is always in one file, the file on screen, and keys act on it as they do in
//! one-file mode. The other files are drawn above and below it. Moving past the first or last
//! line of a file moves the cursor into the next one. Only the files near the screen are read
//! and highlighted.

use std::collections::HashMap;

use glos_helix::{
    commands::{self, SCROLLOFF},
    text::Selection,
};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
};

use super::{
    App, Cur, ViewKind,
    comments::Threads,
    view::{self, File, clean},
};

/// The most file documents kept at once. A file further off screen is read again when it comes
/// back.
pub(super) const MAX_LOADED: usize = 50;

/// A place in the scroll: a file of the review, and a line of its document. Line 0 comes with
/// the file's header and notices above it.
pub(super) type Pos = (usize, usize);

#[derive(Default)]
pub(super) struct AllFiles {
    pub on: bool,
    /// The place at the top of the screen.
    pub top: Pos,
    /// The view of each file drawn that the cursor isn't in, with its old side in the
    /// side-by-side view. It's `None` when the file itself is drawn: in the whole-file view
    /// with nothing folded, or for a binary or collapsed file.
    docs: HashMap<usize, Option<(File, Option<File>)>>,
    /// What `docs` were built with: the generation of the threads and files, the lines of
    /// context and the view. Any of them changing builds them again.
    built: (u64, usize, Option<ViewKind>),
    /// From the last draw, for the mouse: each file's text on screen, with the file and the
    /// line at its top, and each header's row, with its file.
    regions: Vec<(Rect, usize, usize)>,
    headers: Vec<(u16, usize)>,
}

impl AllFiles {
    pub(super) fn new(on: bool) -> AllFiles {
        AllFiles {
            on,
            ..Default::default()
        }
    }
}

impl App {
    /// The screen shows every file: the mode is on, and the cursor is in one of the review's
    /// changes.
    pub(super) fn all_shown(&self) -> bool {
        self.all.on && matches!(self.state.cur, Cur::Change(_))
    }

    pub(super) fn cur_index(&self) -> usize {
        match self.state.cur {
            Cur::Change(i) => i,
            Cur::Context(_) => usize::MAX,
        }
    }

    /// Reads file `i` if it isn't loaded, and builds its view unless the cursor is in it.
    fn prepare(&mut self, i: usize) {
        let built = (self.generation, self.diff_context, Some(self.view_kind));
        if built != self.all.built {
            self.all.docs.clear();
            self.all.built = built;
        }
        self.load(i);
        if i == self.cur_index() {
            // The file under the cursor is drawn from its own view, which can change, so a
            // copy kept from before would go stale.
            self.all.docs.remove(&i);
            return;
        }
        if self.all.docs.contains_key(&i) {
            return;
        }
        let path = self.state.source.changes[i].path.clone();
        let base = self.state.files[i].as_ref().expect("loaded above");
        let whole = self.view_kind == ViewKind::File;
        let doc = self
            .built_for(&Cur::Change(i), &path, base)
            .filter(|b| b.folded || !whole)
            .map(|b| (b.doc, b.left));
        self.all.docs.insert(i, doc);
    }

    /// File `i` as it's drawn. It must be prepared.
    fn section_file(&self, i: usize) -> &File {
        if i == self.cur_index() {
            return self.main_file();
        }
        match self.all.docs.get(&i) {
            Some(Some((doc, _))) => doc,
            _ => self.state.files[i].as_ref().expect("prepared"),
        }
    }

    /// File `i`'s old side, in the side-by-side view. It must be prepared.
    fn section_left(&self, i: usize) -> Option<&File> {
        if i == self.cur_index() {
            return self.split_left();
        }
        match self.all.docs.get(&i) {
            Some(Some((_, left))) => left.as_ref(),
            _ => None,
        }
    }

    /// Whether lines wrap: as set, except side by side.
    fn all_wrap(&self) -> bool {
        self.state.wrap && self.left_view.width == 0
    }

    fn section_threads(&self, i: usize) -> Threads {
        if i == self.cur_index() {
            return self.main_threads();
        }
        let path = &self.state.source.changes[i].path;
        let base = self.state.files[i].as_ref().expect("prepared");
        self.threads_in(path, base, self.section_file(i))
    }

    /// The lines file `i` has in the scroll. A collapsed or binary file has one, which shows
    /// only its header. The empty line after a file's last newline isn't shown, so files
    /// follow one another without a blank numbered line between them.
    fn section_lines(&self, i: usize) -> usize {
        let f = self.section_file(i);
        if f.collapsed || f.binary {
            return 1;
        }
        let t = f.text.slice(..);
        let n = t.len_lines();
        if n > 1 && t.char(t.len_chars() - 1) == '\n' {
            n - 1
        } else {
            n
        }
    }

    /// The rows above file `i`'s first line: a blank row after the file before it, the header
    /// row, the notices under it, and a collapsed file's summary.
    fn header_rows(&self, i: usize) -> usize {
        usize::from(i > 0) + 1 + self.notices(i).count() + usize::from(self.summary(i).is_some())
    }

    /// File `i`'s notices. The one a generated file has says how to expand it, which its
    /// summary says instead.
    fn notices(&self, i: usize) -> impl Iterator<Item = &String> {
        // From the file itself, as the unified and side-by-side views have none of their own.
        let f = self.state.files[i].as_ref().expect("prepared");
        f.notices.iter().filter(|n| *n != view::GENERATED)
    }

    /// The line under a collapsed file's header, in place of its code: how many lines it has.
    fn summary(&self, i: usize) -> Option<String> {
        let f = self.section_file(i);
        if !f.collapsed || f.binary {
            return None;
        }
        let t = f.text.slice(..);
        let mut n = t.len_lines();
        if n > 1 && t.char(t.len_chars() - 1) == '\n' {
            n -= 1;
        }
        let lines = if n == 1 { "line" } else { "lines" };
        // The ▶ in the header already says the file is collapsed.
        let generated = if f.notices.iter().any(|n| n == view::GENERATED) {
            ", generated file"
        } else {
            ""
        };
        Some(format!("↕ {n} {lines}{generated}"))
    }

    /// The rows the pinned header takes with `top` at the top of the screen. While a file's
    /// header has scrolled off, a copy of it stays on the top row.
    fn pinned(top: Pos) -> usize {
        usize::from(top.1 > 0)
    }

    /// The screen rows place `pos` takes: its line, the thread rows under it, and for line 0
    /// the file's header. `threads` keeps each file's threads for the rest of the draw.
    fn rows_at(&mut self, pos: Pos, threads: &mut HashMap<usize, Threads>) -> usize {
        let (i, line) = pos;
        self.prepare(i);
        let mut rows = if line == 0 { self.header_rows(i) } else { 0 };
        let f = self.section_file(i);
        if !(f.collapsed || f.binary) {
            let t = threads.entry(i).or_insert_with(|| self.section_threads(i));
            let width = self.view.width.saturating_sub(f.gutter_width() as u16);
            let fmt = view::text_format(f, width, self.all_wrap());
            rows += view::line_rows(f, line, &fmt, t);
        }
        rows
    }

    fn next_pos(&mut self, (i, line): Pos) -> Option<Pos> {
        self.prepare(i);
        if line + 1 < self.section_lines(i) {
            Some((i, line + 1))
        } else if i + 1 < self.state.files.len() {
            Some((i + 1, 0))
        } else {
            None
        }
    }

    fn prev_pos(&mut self, (i, line): Pos) -> Option<Pos> {
        if line > 0 {
            return Some((i, line - 1));
        }
        let i = i.checked_sub(1)?;
        self.prepare(i);
        Some((i, self.section_lines(i) - 1))
    }

    /// The place `n` lines before `pos`, or the start of the review.
    fn back(&mut self, mut pos: Pos, n: usize) -> Pos {
        for _ in 0..n {
            match self.prev_pos(pos) {
                Some(p) => pos = p,
                None => break,
            }
        }
        pos
    }

    fn cursor_pos(&self) -> Pos {
        let f = self.main_file();
        if f.collapsed || f.binary {
            return (self.cur_index(), 0);
        }
        let t = f.text.slice(..);
        (self.cur_index(), t.char_to_line(f.sel.primary().cursor(t)))
    }

    /// Scrolls so the cursor is on screen with `scrolloff` lines around it. A cursor far off
    /// screen ends up in the middle, as Helix centres after a jump.
    fn follow_cursor(&mut self, height: usize, threads: &mut HashMap<usize, Threads>) {
        let so = SCROLLOFF.min(height.saturating_sub(1) / 2);
        let cur = self.cursor_pos();
        if cur < self.all.top {
            self.all.top = self.back(cur, so);
            return;
        }
        // The rows from the top of the screen to the cursor's line, its header included.
        let (mut rows, mut p) = (Self::pinned(self.all.top), self.all.top);
        while p < cur && rows <= 2 * height {
            rows += self.rows_at(p, threads);
            match self.next_pos(p) {
                Some(q) => p = q,
                None => break,
            }
        }
        if p < cur {
            self.all.top = self.back(cur, height / 2);
            return;
        }
        if cur.1 == 0 {
            rows += self.header_rows(cur.0);
        }
        // A file's header at the top of the screen stays there, as the start of a file does.
        if rows < so && self.all.top > (0, 0) && self.all.top != (cur.0, 0) {
            self.all.top = self.back(cur, so);
            return;
        }
        while rows + so >= height && self.all.top < cur {
            let was = Self::pinned(self.all.top);
            rows -= self.rows_at(self.all.top, threads) + was;
            match self.next_pos(self.all.top) {
                Some(q) => self.all.top = q,
                None => break,
            }
            rows += Self::pinned(self.all.top);
        }
    }

    /// Draws the files from the top of the screen down, and returns the screen row of each
    /// thread row drawn, with its thread.
    pub(super) fn draw_all(
        &mut self,
        buf: &mut Buffer,
        area: Rect,
        styles: view::SelectionStyles,
    ) -> Vec<(u16, usize)> {
        let n = self.state.files.len();
        if n == 0 || area.height == 0 {
            return vec![];
        }
        let mut threads = HashMap::new();
        let (i, line) = self.all.top;
        let i = i.min(n - 1);
        self.prepare(i);
        self.all.top = (i, line.min(self.section_lines(i) - 1));
        self.follow_cursor(area.height as usize, &mut threads);

        let cur = self.cur_index();
        let none = Style::default();
        // Headers go across both columns in the side-by-side view.
        let left_col = self.left_view;
        let area = if left_col.width > 0 {
            Rect::new(left_col.x, area.y, area.right() - left_col.x, area.height)
        } else {
            area
        };
        let right_x = self.view.x;
        let (mut hits, mut drawn) = (vec![], vec![]);
        let (mut regions, mut headers) = (vec![], vec![]);
        let (mut y, (mut i, mut line)) = (area.y, self.all.top);
        if Self::pinned(self.all.top) > 0 {
            headers.push((y, i));
            self.draw_header(buf, area, y, i);
            y += 1;
        }
        while y < area.bottom() && i < n {
            self.prepare(i);
            drawn.push(i);
            let f = self.section_file(i);
            if line == 0 {
                // The blank row between files isn't needed at the top of the screen.
                if i > 0 && y > area.y {
                    y += 1;
                }
                if y >= area.bottom() {
                    break;
                }
                headers.push((y, i));
                self.draw_header(buf, area, y, i);
                y += 1;
                let style = self.theme.get("ui.virtual");
                let mut rows: Vec<(String, u16)> = self
                    .notices(i)
                    .map(|n| (clean(n).into_owned(), 1))
                    .collect();
                rows.extend(self.summary(i).map(|s| (s, 3)));
                for (text, x) in rows {
                    if y >= area.bottom() {
                        break;
                    }
                    // A click on the summary opens the file, as one on the header does.
                    if x == 3 {
                        headers.push((y, i));
                    }
                    let width = area.width.saturating_sub(x) as usize;
                    buf.set_stringn(area.x + x, y, text, width, style);
                    y += 1;
                }
            }
            if !(f.collapsed || f.binary) && y < area.bottom() {
                // Down to the file's last line, so the next file's header follows it.
                let mut height = 0;
                for l in line..self.section_lines(i) {
                    if y + height >= area.bottom() {
                        break;
                    }
                    let header = if l == 0 { self.header_rows(i) } else { 0 };
                    height += (self.rows_at((i, l), &mut threads) - header) as u16;
                }
                let height = height.min(area.bottom() - y);
                let f = self.section_file(i);
                let t = threads.entry(i).or_insert_with(|| self.section_threads(i));
                let (main, left) = if i == cur {
                    (styles, f.left)
                } else {
                    ((none, none, none, none), 0)
                };
                let rest = Rect::new(right_x, y, area.right() - right_x, height);
                regions.push((rest, i, line));
                if let Some(old) = self.section_left(i) {
                    let col = Rect::new(left_col.x, y, left_col.width, height);
                    regions.push((col, i, line));
                    let blank = super::blank_threads(t);
                    view::draw_text(
                        buf,
                        col,
                        old,
                        line,
                        left,
                        false,
                        &self.state.loader,
                        &self.theme,
                        Some((none, none, none, none)),
                        &blank,
                    );
                }
                let (h, end) = view::draw_text(
                    buf,
                    rest,
                    f,
                    line,
                    left,
                    self.all_wrap(),
                    &self.state.loader,
                    &self.theme,
                    Some(main),
                    t,
                );
                hits.extend(h);
                y = end;
            }
            (i, line) = (i + 1, 0);
        }
        (self.all.regions, self.all.headers) = (regions, headers);
        self.trim(&drawn);
        hits
    }

    /// Draws file `i`'s header on row `y`: a rule across the screen with the header's text
    /// set into it.
    fn draw_header(&self, buf: &mut Buffer, area: Rect, y: u16, i: usize) {
        let style = self
            .theme
            .get("ui.linenr.selected")
            .add_modifier(Modifier::BOLD);
        let open = if self.section_file(i).collapsed {
            '▶'
        } else {
            '▼'
        };
        let (text, added, removed, threads) = self.header(i);
        let rule = "─".repeat(area.width as usize);
        let line = Line::from(vec![
            Span::styled(format!("── {open} {text}  "), style),
            Span::styled(added, style.patch(self.theme.get("diff.plus"))),
            Span::styled(" ", style),
            Span::styled(removed, style.patch(self.theme.get("diff.minus"))),
            Span::styled(format!("{threads} {rule}"), style),
        ]);
        buf.set_line(area.x, y, &line, area.width);
    }

    /// A file's header: its path, the lines added and removed, and its open threads.
    fn header(&self, i: usize) -> (String, String, String, String) {
        let path = &self.state.source.changes[i].path;
        let base = self.state.files[i].as_ref().expect("prepared");
        let added: u32 = base.hunks.iter().map(|h| h.after.len() as u32).sum();
        let removed: u32 = base.hunks.iter().map(|h| h.before.len() as u32).sum();
        let mut s = String::new();
        let open = self
            .state
            .threads
            .iter()
            .filter(|t| t.status == "open" && t.anchor.as_ref().is_some_and(|a| &a.path == path))
            .count();
        match open {
            0 => {}
            1 => s += "  1 thread",
            n => s += &format!("  {n} threads"),
        }
        (
            clean(path).into_owned(),
            format!("+{added}"),
            format!("-{removed}"),
            s,
        )
    }

    /// Drops the documents of files far from the screen once more than [`MAX_LOADED`] are
    /// loaded. The file under the cursor and those on the jumplist stay.
    fn trim(&mut self, drawn: &[usize]) {
        let loaded = self.state.files.iter().filter(|f| f.is_some()).count();
        if loaded <= MAX_LOADED {
            return;
        }
        let cur = self.cur_index();
        for i in 0..self.state.files.len() {
            // A file the reviewer collapsed or expanded stays loaded, so it stays that way.
            let toggled = self.state.files[i].as_ref().is_some_and(|f| {
                f.collapsed != self.state.source.collapsed(&self.state.source.changes[i])
            });
            let keep = i == cur
                || toggled
                || drawn.contains(&i)
                || self.jumps.iter().any(|(c, _)| *c == Cur::Change(i));
            if !keep {
                self.state.files[i] = None;
                self.all.docs.remove(&i);
            }
        }
    }

    /// Moves the cursor into file `i`, on its first line or its last.
    pub(super) fn enter_file(&mut self, i: usize, last: bool) {
        self.load(i);
        self.state.cur = Cur::Change(i);
        self.ensure_alt();
        let line = if last { self.section_lines(i) - 1 } else { 0 };
        self.cursor_to_line(line);
    }

    /// Puts the cursor at the start of line `line` of the file on screen, or its last line.
    pub(super) fn cursor_to_line(&mut self, line: usize) {
        let f = self.main_file_mut();
        let t = f.text.slice(..);
        let line = line.min(t.len_lines() - 1);
        f.sel = Selection::point(t.line_to_char(line)).ensure_invariants(t);
    }

    /// The keys that move past the file in all-files mode: `j` and `k` at its last and first
    /// lines, `gg` and `G`, and the page keys. Returns whether `name` was handled here.
    pub(super) fn all_files_motion(&mut self, name: &str) -> bool {
        let n = self.state.files.len();
        let (i, line) = self.cursor_pos();
        self.prepare(i);
        let last = self.section_lines(i) - 1;
        let count = self.count.map_or(1, |c| c.get());
        let height = self.view.height as usize;
        let so = SCROLLOFF.min(height.saturating_sub(1) / 2);
        match name {
            // A count that goes past the file carries on into the next one.
            "move_visual_line_down" | "move_line_down" if line + count > last && i + 1 < n => {
                self.move_lines(count, true)
            }
            "move_visual_line_up" | "move_line_up" if count > line && i > 0 => {
                self.move_lines(count, false)
            }
            "scroll_down" | "scroll_up" => self.all_wheel(count, name == "scroll_down"),
            "align_view_center" => {
                self.all.top = self.back_rows(self.cursor_pos(), height / 2);
            }
            "align_view_top" => self.all.top = self.back_rows(self.cursor_pos(), so),
            "align_view_bottom" => {
                let rows = height.saturating_sub(so + 1);
                self.all.top = self.back_rows(self.cursor_pos(), rows);
            }
            "goto_window_top" | "goto_window_center" | "goto_window_bottom" => {
                let (first, middle, last) = self.on_screen();
                let to = match name {
                    "goto_window_top" => first,
                    "goto_window_center" => middle,
                    _ => last,
                };
                if to.0 != self.cur_index() {
                    self.enter_file(to.0, false);
                }
                self.cursor_to_line(to.1);
            }
            "goto_file_start" if self.count.is_none() => {
                self.enter_file(0, false);
                self.all.top = (0, 0);
            }
            "goto_line" if self.count.is_none() => self.enter_file(n - 1, true),
            "page_down" | "page_up" | "page_cursor_half_down" | "page_cursor_half_up" => {
                let rows = self.view.height as usize;
                let rows = if name.contains("half") {
                    rows / 2
                } else {
                    rows
                };
                self.page(rows.max(1), name.ends_with("down"));
            }
            _ => return false,
        }
        true
    }

    /// Moves the cursor and the screen `n` lines through the review.
    fn page(&mut self, n: usize, down: bool) {
        let (mut p, mut top) = (self.cursor_pos(), self.all.top);
        for _ in 0..n {
            let step = |app: &mut App, pos| {
                if down {
                    app.next_pos(pos)
                } else {
                    app.prev_pos(pos)
                }
            };
            match step(self, p) {
                Some(q) => p = q,
                None => break,
            }
            if let Some(t) = step(self, top) {
                top = t;
            }
        }
        if p.0 != self.cur_index() {
            self.enter_file(p.0, false);
        }
        self.cursor_to_line(p.1);
        self.all.top = top;
    }

    /// Moves the cursor `n` lines through the review, into other files if need be.
    fn move_lines(&mut self, n: usize, down: bool) {
        let mut p = self.cursor_pos();
        for _ in 0..n {
            let next = if down {
                self.next_pos(p)
            } else {
                self.prev_pos(p)
            };
            match next {
                Some(q) => p = q,
                None => break,
            }
        }
        if p.0 != self.cur_index() {
            self.enter_file(p.0, false);
        }
        self.cursor_to_line(p.1);
    }

    /// The place to put at the top of the screen so that `rows` screen rows are above `pos`.
    fn back_rows(&mut self, pos: Pos, rows: usize) -> Pos {
        let mut threads = HashMap::new();
        self.prepare(pos.0);
        let (mut p, mut above) = (
            pos,
            if pos.1 == 0 {
                self.header_rows(pos.0)
            } else {
                0
            },
        );
        while let Some(q) = self.prev_pos(p) {
            let r = self.rows_at(q, &mut threads);
            if above + r > rows {
                break;
            }
            above += r;
            p = q;
        }
        p
    }

    /// The first, middle and last places the cursor may be with the screen where it is, as
    /// `follow_cursor` keeps it.
    fn on_screen(&mut self) -> (Pos, Pos, Pos) {
        let height = self.view.height as usize;
        let so = SCROLLOFF.min(height.saturating_sub(1) / 2);
        let mut threads = HashMap::new();
        let top = self.all.top;
        let first = if top.1 == 0 {
            top
        } else {
            let mut p = top;
            for _ in 0..so {
                match self.next_pos(p) {
                    Some(q) => p = q,
                    None => break,
                }
            }
            p
        };
        let (mut rows, mut p, mut last, mut middle) = (Self::pinned(top), top, top, top);
        loop {
            self.prepare(p.0);
            let header = if p.1 == 0 { self.header_rows(p.0) } else { 0 };
            if rows + header + so >= height && p > top {
                break;
            }
            last = p;
            if rows + header <= height / 2 {
                middle = p;
            }
            rows += self.rows_at(p, &mut threads);
            match self.next_pos(p) {
                Some(q) => p = q,
                None => break,
            }
        }
        (first, middle.max(first).min(last), last)
    }

    /// The mouse wheel: scrolls the review `n` lines, and moves the cursor only if it would
    /// leave the screen, into another file if need be.
    pub(super) fn all_wheel(&mut self, n: usize, down: bool) {
        for _ in 0..n {
            let top = self.all.top;
            let next = if down {
                self.next_pos(top)
            } else {
                self.prev_pos(top)
            };
            match next {
                Some(p) => self.all.top = p,
                None => break,
            }
        }
        let (first, _, last) = self.on_screen();
        let cur = self.cursor_pos();
        let to = if cur < first {
            first
        } else if cur > last {
            last
        } else {
            return;
        };
        if to.0 != self.cur_index() {
            self.enter_file(to.0, false);
        }
        self.cursor_to_line(to.1);
    }

    /// Before a click or drag in all-files mode: moves the cursor into the file under the
    /// pointer, so the click lands in it. A click on a header collapses or expands its file.
    /// Returns whether the click lands on text; a drag stays in the file it started in.
    pub(super) fn all_focus_at(&mut self, x: u16, y: u16, drag: bool) -> bool {
        if let Some(&(_, i)) = self.all.headers.iter().find(|(hy, _)| *hy == y) {
            if !drag {
                self.toggle_file(i);
            }
            return false;
        }
        let Some(&(_, i, _)) = self
            .all
            .regions
            .iter()
            .find(|(r, ..)| r.contains((x, y).into()))
        else {
            return false;
        };
        if i != self.cur_index() {
            if drag {
                return false;
            }
            self.enter_file(i, false);
        }
        true
    }

    /// Collapses file `i` to its header, or expands it. Its folds stay as they were, and so
    /// does the cursor in it.
    pub(super) fn toggle_file(&mut self, i: usize) {
        self.load(i);
        let f = self.state.files[i].as_mut().expect("loaded above");
        f.collapsed = !f.collapsed;
        self.all.docs.remove(&i);
        if i == self.cur_index() {
            self.refold(None);
        }
    }

    /// The text area and the top line under row `y`, in the file under the cursor.
    pub(super) fn all_region_at(&self, y: u16) -> Option<(Rect, usize)> {
        let cur = self.cur_index();
        self.all
            .regions
            .iter()
            .find(|(r, i, _)| *i == cur && r.y <= y && y < r.bottom())
            .map(|&(r, _, line)| (r, line))
    }

    /// `' v a`, `:all` and `:one`: shows every file, or the file under the cursor alone. The
    /// cursor stays where it was, and so does the screen as far as it can.
    pub(super) fn set_all_files(&mut self, on: bool) {
        let was = self.all_shown();
        self.all.on = on;
        if self.all_shown() && !was {
            let (i, _) = self.cursor_pos();
            self.all.top = (i, self.main_file().top);
        } else if was && !self.all_shown() {
            let (i, line) = self.all.top;
            let top = if i == self.cur_index() { line } else { 0 };
            self.main_file_mut().top = top;
            self.scroll_to_cursor(false);
        }
        let status = if on {
            "All files"
        } else {
            "One file at a time"
        };
        self.status = Some((status.into(), false));
    }

    /// Puts the cursor back in file `i`, where it was in that file, with `top` at the top of
    /// the screen.
    pub(super) fn back_to_file(&mut self, i: usize, top: Pos) {
        if i != self.cur_index() {
            self.load(i);
            self.state.cur = Cur::Change(i);
            self.ensure_alt();
        }
        self.all.top = top;
    }

    /// A jump that goes on past the cursor's file. `jump` runs in the cursor's file from the
    /// cursor, and says whether it found something there. If it didn't, `jump` runs in each
    /// file after the cursor's (or before it, going back) for which `want` holds, told that
    /// it starts from the file's start (or end). With `wrap`, it goes round past the last
    /// file to the first. Returns whether it wrapped, or `None`, with the cursor back where it
    /// was, when no file had anything.
    fn jump_across(
        &mut self,
        forward: bool,
        wrap: bool,
        want: impl Fn(&App, usize) -> bool,
        mut jump: impl FnMut(&mut App, bool) -> bool,
    ) -> Option<bool> {
        let (i, sel, top) = (self.cur_index(), self.main_file().sel.clone(), self.all.top);
        if jump(self, false) {
            return Some(false);
        }
        let n = self.state.files.len();
        for k in 1..=n {
            let wrapped = if forward { i + k >= n } else { k > i };
            if wrapped && !wrap {
                break;
            }
            let j = if forward {
                (i + k) % n
            } else {
                (i + n - k) % n
            };
            if !want(self, j) {
                continue;
            }
            self.enter_file(j, !forward);
            if jump(self, true) {
                return Some(wrapped);
            }
        }
        self.back_to_file(i, top);
        self.main_file_mut().sel = sel;
        None
    }

    /// `n`, `N` and a search as it's typed: the next match of `query`, in a later file if
    /// there are no more in this one, and round to the first file after the last. With
    /// `warn`, the status line says when the search wraps or finds nothing.
    pub(super) fn search_across(&mut self, query: &str, forward: bool, warn: bool) {
        let Some(regex) = commands::search_regex(query) else {
            return;
        };
        for _ in 0..self.count.map_or(1, |c| c.get()) {
            let found = self.jump_across(
                forward,
                true,
                |app, j| app.may_match(j, &regex),
                |app, entered| app.search_in_file(&regex, forward, entered),
            );
            if warn {
                self.status = match found {
                    Some(false) => None,
                    Some(true) => Some(("Wrapped around the review".into(), false)),
                    None => Some(("No more matches".into(), true)),
                };
            }
            if found.is_none() {
                break;
            }
        }
    }

    /// Whether file `j`'s text has a match for `regex`, checked before the file is opened,
    /// because opening every file of a big review to search it takes seconds. A collapsed or
    /// binary file can't show a match.
    fn may_match(&self, j: usize, regex: &glos_helix::text::Regex) -> bool {
        use glos_helix::text::{Rope, RopeSliceExt};
        if let Some(f) = &self.state.files[j] {
            return !(f.collapsed || f.binary)
                && regex.find(f.text.slice(..).regex_input()).is_some();
        }
        use crate::source::Content;
        let (source, change) = (&self.state.source, &self.state.source.changes[j]);
        if source.collapsed(change) {
            return false;
        }
        let shown = if change.new == Content::None {
            &change.old
        } else {
            &change.new
        };
        let Some(bytes) = source.read(shown).ok().flatten() else {
            return false;
        };
        let text = Rope::from(String::from_utf8_lossy(&bytes).as_ref());
        regex.find(text.slice(..).regex_input()).is_some()
    }

    /// Selects the next match of `regex` in the file on screen without wrapping round it, or
    /// with `entered` its first match (its last, going back). Returns whether there was one.
    fn search_in_file(
        &mut self,
        regex: &glos_helix::text::Regex,
        forward: bool,
        entered: bool,
    ) -> bool {
        let f = self.main_file_mut();
        let before = f.sel.clone();
        if entered {
            // From the end of the file a search forward wraps round to its first match, and
            // from the start a search back wraps round to its last.
            let t = f.text.slice(..);
            let at = if forward {
                t.len_chars().saturating_sub(1)
            } else {
                0
            };
            f.sel = Selection::point(at).ensure_invariants(t);
        }
        let from = self.main_file().sel.clone();
        self.doc_command(|cx| commands::search(cx, regex, forward, false));
        let (old, new) = (from.primary().from(), self.main_file().sel.primary().from());
        let found = if entered {
            self.main_file().sel != from
        } else if forward {
            new > old
        } else {
            new < old
        };
        if !found {
            self.main_file_mut().sel = before;
        }
        found
    }

    /// The motions that look for the next thing of a kind, `]g` and `]p` and the rest, carry on
    /// into the next file when there's nothing more in this one. Returns whether `name` was
    /// one of them.
    pub(super) fn jump_across_files(&mut self, name: &str, f: fn(&mut commands::Ctx)) -> bool {
        match name {
            "goto_next_change" | "goto_prev_change" => {
                self.change_across(name == "goto_next_change", f)
            }
            "goto_next_paragraph" | "goto_prev_paragraph" => {
                self.paragraph_across(name == "goto_next_paragraph", f)
            }
            _ => return false,
        }
        true
    }

    /// The cursor's place in the file on screen, to tell which way a motion moved it.
    fn cursor_char(&self) -> usize {
        let f = self.main_file();
        f.sel.primary().cursor(f.text.slice(..))
    }

    /// `]p` and `[p`: the next paragraph, in the next file if this one has no more. A file's
    /// first line starts its first paragraph.
    fn paragraph_across(&mut self, forward: bool, next: fn(&mut commands::Ctx)) {
        let found = self.jump_across(
            forward,
            false,
            |_, _| true,
            |app, entered| {
                if entered && forward {
                    return true;
                }
                let before = app.cursor_char();
                app.doc_command(next);
                let after = app.cursor_char();
                entered
                    || if forward {
                        after > before
                    } else {
                        after < before
                    }
            },
        );
        if found.is_none() {
            self.status = Some(("No more paragraphs".into(), false));
        }
    }

    /// `]f`, `[f` and the other syntax objects: the next one, in the next file if this one has
    /// no more.
    pub(super) fn object_across(&mut self, forward: bool, ch: char) {
        let found = self.jump_across(
            forward,
            false,
            |_, _| true,
            |app, entered| {
                let before = app.main_file().sel.clone();
                app.status = None;
                // The search for the next object starts after the cursor, so an object at the
                // very start of a file is found by selecting the one around the cursor.
                if entered && forward {
                    app.ts_object(true, ch, false);
                    if app.main_file().sel != before && app.main_file().sel.primary().from() == 0 {
                        return true;
                    }
                    app.main_file_mut().sel = before.clone();
                }
                // Going back, the search starts from the file's very end, so an object on its
                // last line is found.
                if entered && !forward {
                    let f = app.main_file_mut();
                    let t = f.text.slice(..);
                    f.sel = Selection::point(t.len_chars());
                }
                app.ts_object(forward, ch, true);
                app.main_file().sel != before
            },
        );
        if found.is_none() {
            self.status = Some(("No more to go to".into(), false));
        }
    }

    /// `]g` and `[g`: the next change, in the next file if this one has no more.
    fn change_across(&mut self, forward: bool, next: fn(&mut commands::Ctx)) {
        let found = self.jump_across(
            forward,
            false,
            |_, _| true,
            |app, entered| {
                let before = app.main_file().sel.clone();
                app.status = None;
                match (entered, forward) {
                    (false, _) => app.doc_command(next),
                    (true, true) => app.doc_command(commands::goto_first_change),
                    (true, false) => app.doc_command(commands::goto_last_change),
                }
                app.main_file().sel != before
            },
        );
        if found.is_none() {
            self.status = Some(("No more changes".into(), false));
        }
    }

    /// `]r`, `[r`, `]R` and `[R`: the next thread, in the next file if this one has no more;
    /// or the last or first thread of the review.
    pub(super) fn thread_across(&mut self, forward: bool, edge: bool) {
        let has_threads = |app: &App, j: usize| {
            let path = &app.state.source.changes[j].path;
            app.state
                .threads
                .iter()
                .any(|t| t.anchor.as_ref().is_some_and(|a| &a.path == path))
        };
        if edge {
            // The last thread is the last one in the last file that has any.
            let n = self.state.files.len();
            let order: Vec<usize> = if forward {
                (0..n).rev().collect()
            } else {
                (0..n).collect()
            };
            for j in order {
                if has_threads(self, j) {
                    self.enter_file(j, forward);
                    if self.goto_thread_in(forward, true) {
                        return;
                    }
                }
            }
            self.status = Some(("No threads in this review".into(), false));
            return;
        }
        let found = self.jump_across(forward, false, has_threads, |app, entered| {
            // Entering a file going forward, its first thread is next, and going back its
            // last.
            if entered {
                app.goto_thread_in(!forward, true)
            } else {
                app.goto_thread_in(forward, false)
            }
        });
        if found.is_none() {
            self.status = Some(("No more threads".into(), false));
        }
    }
}
