//! The file tree (`space e`): the review's changed files by folder, each with the lines it adds
//! and removes and its open threads.

use crossterm::event::{KeyCode, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{buffer::Buffer, layout::Rect};
use unicode_width::UnicodeWidthStr;

use super::{
    component::{Component, Cx, Msg, Outcome},
    keymap::Key,
    picker::Target,
    view::clean,
};
use crate::theme::Theme;

/// A changed file as the tree lists it.
pub struct Entry {
    pub path: String,
    pub added: usize,
    pub removed: usize,
    pub threads: usize,
}

pub struct Row {
    /// The name as the tree shows it: `mod.rs` for a file, `ui/` for a folder.
    pub name: String,
    /// The full path. A folder's ends in `/`, as in `src/ui/`.
    pub path: String,
    /// How many folders it's inside.
    pub depth: usize,
    pub added: usize,
    pub removed: usize,
    pub threads: usize,
}

pub struct Tree {
    pub rows: Vec<Row>,
    /// The selected row.
    pub cursor: usize,
    /// Keys go to the tree while it's focused; otherwise it's only shown.
    pub focused: bool,
    /// The first row shown below the title. The wheel changes it without moving the cursor.
    pub top: usize,
    /// Set when the cursor moves, so that the next draw scrolls to show it.
    follow: bool,
}

impl Row {
    pub fn is_folder(&self) -> bool {
        self.path.ends_with('/')
    }

    /// The file's path; `None` for a folder.
    pub fn file(&self) -> Option<&str> {
        (!self.is_folder()).then_some(self.path.as_str())
    }
}

impl Tree {
    /// Folders first-seen in path order, each file under its folder, indented by depth.
    pub fn new(mut entries: Vec<Entry>) -> Tree {
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        let mut rows = vec![];
        let mut open: Vec<&str> = vec![];
        for e in &entries {
            let parts: Vec<&str> = e.path.split('/').collect();
            let (dirs, name) = parts.split_at(parts.len() - 1);
            let shared = open.iter().zip(dirs).take_while(|(a, b)| a == b).count();
            open.truncate(shared);
            for (depth, d) in dirs.iter().enumerate().skip(shared) {
                rows.push(Row {
                    name: format!("{d}/"),
                    path: format!("{}/", dirs[..=depth].join("/")),
                    depth,
                    added: 0,
                    removed: 0,
                    threads: 0,
                });
                open.push(d);
            }
            rows.push(Row {
                name: name[0].to_string(),
                path: e.path.clone(),
                depth: dirs.len(),
                added: e.added,
                removed: e.removed,
                threads: e.threads,
            });
        }
        let cursor = rows.iter().position(|r| !r.is_folder()).unwrap_or(0);
        Tree {
            rows,
            cursor,
            focused: true,
            top: 0,
            follow: true,
        }
    }

    /// Puts the cursor on `path`'s row, if it's listed.
    pub fn select(&mut self, path: &str) {
        if let Some(i) = self.rows.iter().position(|r| r.file() == Some(path)) {
            self.cursor = i;
            self.follow = true;
        }
    }

    /// `j`/`k`: to the next or previous file, skipping folders.
    pub fn step(&mut self, down: bool) {
        let files = |i: &usize| !self.rows[*i].is_folder();
        let next = if down {
            (self.cursor + 1..self.rows.len()).find(files)
        } else {
            (0..self.cursor).rev().find(files)
        };
        if let Some(i) = next {
            self.cursor = i;
            self.follow = true;
        }
    }

    pub fn selected(&self) -> Option<&str> {
        self.rows.get(self.cursor)?.file()
    }

    /// The file on the screen row `y` of `area`, as last drawn.
    pub fn path_at(&self, area: Rect, y: u16) -> Option<&str> {
        let i = self.top + y.checked_sub(area.y + 1)? as usize;
        self.rows.get(i)?.file()
    }

    fn render_in(&mut self, buf: &mut Buffer, area: Rect, current: &str, theme: &Theme) {
        let base = theme.get("ui.background");
        buf.set_style(area, base);
        let title = theme.get("ui.text").patch(ratatui::style::Style::default());
        buf.set_stringn(area.x, area.y, " Changed files", area.width as usize, title);
        let height = area.height.saturating_sub(1) as usize;
        if std::mem::take(&mut self.follow) {
            if self.cursor < self.top {
                self.top = self.cursor;
            } else if height > 0 && self.cursor >= self.top + height {
                self.top = self.cursor + 1 - height;
            }
        }
        self.top = self.top.min(self.rows.len().saturating_sub(height));
        let w = area.width.saturating_sub(1) as usize; // the last column separates it
        for (row, r) in self.rows.iter().enumerate().skip(self.top).take(height) {
            let y = area.y + 1 + (row - self.top) as u16;
            let mut style = theme.get(if !r.is_folder() {
                "ui.text"
            } else {
                "ui.text.directory"
            });
            if r.file() == Some(current) {
                style = style.patch(theme.get("ui.text.focus"));
            }
            if row == self.cursor && self.focused {
                style = style.patch(theme.get("ui.menu.selected"));
                buf.set_style(Rect::new(area.x, y, w as u16, 1), style);
            }
            // Right-aligned: +added -removed and a thread badge.
            let mut tail = vec![];
            if !r.is_folder() {
                if r.added > 0 {
                    tail.push((format!("+{}", r.added), theme.get("diff.plus")));
                }
                if r.removed > 0 {
                    tail.push((format!("-{}", r.removed), theme.get("diff.minus")));
                }
                if r.threads > 0 {
                    tail.push((format!("●{}", r.threads), theme.get("info")));
                }
            }
            let tail_w: usize = tail.iter().map(|(t, _)| t.width() + 1).sum();
            let name = format!("{}{}", "  ".repeat(r.depth), clean(&r.name));
            buf.set_stringn(area.x + 1, y, &name, w.saturating_sub(tail_w + 2), style);
            let mut x = area.x as usize + w.saturating_sub(tail_w);
            for (t, s) in tail {
                if x + t.width() <= area.x as usize + w {
                    buf.set_string(x as u16, y, &t, style.patch(s));
                }
                x += t.width() + 1;
            }
        }
        for y in area.y..area.bottom() {
            buf.set_string(
                area.right().saturating_sub(1),
                y,
                "│",
                theme.get("ui.window"),
            );
        }
    }
}

impl Component for Tree {
    /// Every key is the tree's while it's focused.
    fn key(&mut self, key: Key, _cx: &Cx) -> Outcome {
        match (key.code, key.char()) {
            (KeyCode::Down, _) | (_, Some('j')) => self.step(true),
            (KeyCode::Up, _) | (_, Some('k')) => self.step(false),
            (KeyCode::Enter, _) | (_, Some('l')) => {
                if let Some(p) = self.selected().map(str::to_string) {
                    self.focused = false;
                    return Outcome::Msg(Msg::Open(Target::Path(p)));
                }
            }
            (KeyCode::Esc, _) => self.focused = false,
            (_, Some('q')) => return Outcome::Msg(Msg::CloseTree),
            _ => {}
        }
        Outcome::Consumed
    }

    /// A click anywhere in the tree focuses it, and a click on a file also opens that file.
    /// The wheel scrolls the rows without moving the cursor.
    fn mouse(&mut self, m: MouseEvent, area: Rect, _cx: &Cx) -> Outcome {
        if !area.contains((m.column, m.row).into()) {
            return Outcome::Ignored;
        }
        match m.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                self.focused = true;
                if let Some(p) = self.path_at(area, m.row).map(str::to_string) {
                    self.select(&p);
                    return Outcome::Msg(Msg::Open(Target::Path(p)));
                }
            }
            MouseEventKind::ScrollDown => {
                let height = area.height.saturating_sub(1) as usize;
                self.top = (self.top + 3).min(self.rows.len().saturating_sub(height));
            }
            MouseEventKind::ScrollUp => self.top = self.top.saturating_sub(3),
            _ => {}
        }
        Outcome::Consumed
    }

    /// Draws the tree in `area`, the file on screen marked.
    fn render(&mut self, buf: &mut Buffer, area: Rect, cx: &Cx) {
        let current = cx.state.map_or("", |s| s.main_path());
        self.render_in(buf, area, current, cx.theme);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(path: &str, added: usize, removed: usize, threads: usize) -> Entry {
        Entry {
            path: path.into(),
            added,
            removed,
            threads,
        }
    }

    #[test]
    fn folders_group_their_files() {
        let t = Tree::new(vec![
            e("src/ui/mod.rs", 3, 1, 2),
            e("README.md", 1, 0, 0),
            e("src/main.rs", 0, 4, 0),
            e("src/ui/view.rs", 1, 1, 0),
            e("tests/mcp.rs", 9, 0, 0),
        ]);
        let rows: Vec<(usize, &str, &str)> = t
            .rows
            .iter()
            .map(|r| (r.depth, r.name.as_str(), r.path.as_str()))
            .collect();
        assert_eq!(
            rows,
            [
                (0, "README.md", "README.md"),
                (0, "src/", "src/"),
                (1, "main.rs", "src/main.rs"),
                (1, "ui/", "src/ui/"),
                (2, "mod.rs", "src/ui/mod.rs"),
                (2, "view.rs", "src/ui/view.rs"),
                (0, "tests/", "tests/"),
                (1, "mcp.rs", "tests/mcp.rs")
            ]
        );
        assert_eq!(t.selected(), Some("README.md"));
    }

    #[test]
    fn moving_skips_folders_and_stops_at_the_ends() {
        let mut t = Tree::new(vec![e("a/x", 0, 0, 0), e("b/y", 0, 0, 0)]);
        assert_eq!(
            t.selected(),
            Some("a/x"),
            "starts on the first file, not a folder"
        );
        t.step(true);
        assert_eq!(t.selected(), Some("b/y"));
        t.step(true);
        assert_eq!(t.selected(), Some("b/y"));
        t.step(false);
        t.step(false);
        assert_eq!(t.selected(), Some("a/x"));
        t.select("b/y");
        assert_eq!(t.selected(), Some("b/y"));
        t.select("nope");
        assert_eq!(t.selected(), Some("b/y"));
        assert!(Tree::new(vec![]).selected().is_none());
    }

    #[test]
    fn rows_show_counts_and_badges() {
        let theme = Theme::load("default", crate::theme::ColorSupport::TrueColor).unwrap();
        let mut t = Tree::new(vec![e("src/a.rs", 12, 3, 2), e("b.rs", 0, 0, 0)]);
        let area = Rect::new(0, 0, 30, 5);
        let mut buf = Buffer::empty(area);
        t.render_in(&mut buf, area, "b.rs", &theme);
        let row = |y| {
            (0..30)
                .map(|x| buf[(x, y)].symbol())
                .collect::<String>()
                .trim_end()
                .to_string()
        };
        assert_eq!(row(0).trim_end_matches('│').trim_end(), " Changed files");
        assert!(row(1).starts_with(" b.rs"));
        assert!(
            row(3).contains("a.rs") && row(3).contains("+12 -3 ●2"),
            "{}",
            row(3)
        );
        assert_eq!(t.path_at(area, 3), Some("src/a.rs"));
        assert_eq!(t.path_at(area, 2), None, "a folder");
        assert_eq!(t.path_at(area, 0), None, "the title");
    }

    #[test]
    fn as_a_component() {
        use crossterm::event::KeyModifiers;
        let theme =
            crate::theme::Theme::load("default", crate::theme::ColorSupport::TrueColor).unwrap();
        let cx = Cx {
            screen: Rect::new(0, 0, 80, 24),
            theme: &theme,
            state: None,
        };
        let key = |c| Key {
            code: KeyCode::Char(c),
            mods: KeyModifiers::NONE,
        };
        let mut t = Tree::new(vec![e("a.rs", 1, 0, 0), e("src/b.rs", 1, 0, 0)]);
        assert_eq!(t.key(key('j'), &cx), Outcome::Consumed);
        assert_eq!(t.selected(), Some("src/b.rs"));
        assert_eq!(
            t.key(key('x'), &cx),
            Outcome::Consumed,
            "every key is the tree's"
        );
        assert_eq!(
            t.key(key('l'), &cx),
            Outcome::Msg(Msg::Open(Target::Path("src/b.rs".into())))
        );
        assert!(!t.focused, "opening a file gives the keys back to it");
        assert_eq!(t.key(key('q'), &cx), Outcome::Msg(Msg::CloseTree));

        // Rows: the title, then a.rs, src/, b.rs.
        let area = Rect::new(0, 0, 20, 10);
        let click = |column, row| MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column,
            row,
            modifiers: KeyModifiers::NONE,
        };
        assert_eq!(
            t.mouse(click(3, 1), area, &cx),
            Outcome::Msg(Msg::Open(Target::Path("a.rs".into())))
        );
        assert_eq!(t.selected(), Some("a.rs"));
        assert!(t.focused, "a click focuses the tree, and it keeps focus");
        assert_eq!(
            t.mouse(click(3, 2), area, &cx),
            Outcome::Consumed,
            "a folder"
        );
        assert_eq!(
            t.mouse(click(30, 1), area, &cx),
            Outcome::Ignored,
            "beside the tree"
        );
    }

    #[test]
    fn the_wheel_scrolls_apart_from_the_cursor() {
        use crossterm::event::KeyModifiers;
        let theme = Theme::load("default", crate::theme::ColorSupport::TrueColor).unwrap();
        let cx = Cx {
            screen: Rect::new(0, 0, 80, 24),
            theme: &theme,
            state: None,
        };
        let files: Vec<Entry> = (0..20).map(|i| e(&format!("f{i:02}"), 0, 0, 0)).collect();
        let mut t = Tree::new(files);
        // The area has room for the title and 5 rows.
        let area = Rect::new(0, 0, 20, 6);
        let mut buf = Buffer::empty(area);
        let wheel = |kind| MouseEvent {
            kind,
            column: 2,
            row: 3,
            modifiers: KeyModifiers::NONE,
        };
        t.render_in(&mut buf, area, "", &theme);
        t.mouse(wheel(MouseEventKind::ScrollDown), area, &cx);
        t.mouse(wheel(MouseEventKind::ScrollDown), area, &cx);
        t.render_in(&mut buf, area, "", &theme);
        assert_eq!(
            (t.top, t.selected()),
            (6, Some("f00")),
            "the cursor stays put"
        );
        for _ in 0..10 {
            t.mouse(wheel(MouseEventKind::ScrollDown), area, &cx);
        }
        assert_eq!(t.top, 15, "no further than the last row");
        t.mouse(wheel(MouseEventKind::ScrollUp), area, &cx);
        assert_eq!(t.top, 12);
        // Moving the cursor brings it back into view.
        t.step(true);
        t.render_in(&mut buf, area, "", &theme);
        assert_eq!((t.top, t.selected()), (1, Some("f01")));
        assert_eq!(
            t.mouse(
                wheel(MouseEventKind::ScrollDown),
                Rect::new(30, 0, 20, 6),
                &cx
            ),
            Outcome::Ignored,
            "outside the tree"
        );
    }
}
