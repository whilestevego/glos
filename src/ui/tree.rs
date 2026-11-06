//! The file tree (`space e`): the review's changed files by folder, each with the lines it adds
//! and removes and its open threads.

use std::collections::HashSet;

use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::{buffer::Buffer, layout::Rect};
use unicode_width::UnicodeWidthStr;

use super::{
    component::{Component, Cx, Msg, Outcome},
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

/// What the tree's keys do. `keymap.rs` binds them.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TreeCmd {
    Next,
    Prev,
    /// `ret`: open the file, or open or close the folder.
    Open,
    /// `l`: open the file, or open the folder, or go into it if it's open.
    OpenOrInto,
    /// `h`: close the folder, or go to the folder it's in.
    CloseOrUp,
    /// `/` (`true`) or `?`.
    Search(bool),
    /// `n` (`true`) or `N`.
    SearchNext(bool),
    /// `esc`: the keys go back to the file.
    Unfocus,
    Close,
}

/// Where the tree was when a search started, so `esc` can put it back.
#[derive(Clone)]
pub struct Origin {
    cursor: usize,
    top: usize,
    collapsed: HashSet<String>,
}

/// A search pattern, with the smart case of the code's search: it ignores case unless it has
/// an uppercase letter.
pub fn search_regex(query: &str) -> Option<regex::Regex> {
    regex::RegexBuilder::new(query)
        .case_insensitive(!query.chars().any(char::is_uppercase))
        .build()
        .ok()
}

pub struct Tree {
    pub rows: Vec<Row>,
    /// The selected row, an index into `rows`. It's always on a visible row.
    pub cursor: usize,
    /// Keys go to the tree while it's focused; otherwise it's only shown.
    pub focused: bool,
    /// The first visible row shown below the title. The wheel changes it without moving the
    /// cursor.
    pub top: usize,
    /// The paths of closed folders. Rows inside them aren't shown.
    pub collapsed: HashSet<String>,
    /// Set when the cursor moves, so that the next draw scrolls to show it.
    follow: bool,
    /// The file on screen when the tree was last drawn. When it changes, the folders holding
    /// the new one open.
    shown: String,
}

impl Row {
    pub fn is_folder(&self) -> bool {
        self.path.ends_with('/')
    }

    /// The file's path; `None` for a folder.
    pub fn file(&self) -> Option<&str> {
        (!self.is_folder()).then_some(self.path.as_str())
    }

    /// Whether this row is somewhere inside the folder `folder`.
    fn inside(&self, folder: &str) -> bool {
        self.path.len() > folder.len() && self.path.starts_with(folder)
    }
}

impl Tree {
    /// Folders first-seen in path order, each file under its folder. A folder's counts are the
    /// totals of the files under it, so a closed folder still says what's in it.
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
                let path = format!("{}/", dirs[..=depth].join("/"));
                let under = || entries.iter().filter(|e| e.path.starts_with(&path));
                rows.push(Row {
                    name: format!("{d}/"),
                    depth,
                    added: under().map(|e| e.added).sum(),
                    removed: under().map(|e| e.removed).sum(),
                    threads: under().map(|e| e.threads).sum(),
                    path,
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
            collapsed: HashSet::new(),
            follow: true,
            shown: String::new(),
        }
    }

    /// A new tree over `entries` that keeps this one's focus, scroll, cursor and closed
    /// folders. Closed folders that are gone are dropped.
    pub fn rebuilt(&self, entries: Vec<Entry>) -> Tree {
        let mut t = Tree::new(entries);
        t.collapsed = self
            .collapsed
            .iter()
            .filter(|c| t.rows.iter().any(|r| &&r.path == c))
            .cloned()
            .collect();
        (t.focused, t.top, t.shown) = (self.focused, self.top, self.shown.clone());
        if let Some(r) = self.rows.get(self.cursor) {
            t.select(&r.path);
        }
        t
    }

    /// The rows not inside a closed folder, as indexes into `rows`.
    pub fn visible(&self) -> Vec<usize> {
        (0..self.rows.len())
            .filter(|&i| !self.collapsed.iter().any(|c| self.rows[i].inside(c)))
            .collect()
    }

    /// Moves the cursor to row `i`, opening the folders it's in.
    fn go(&mut self, i: usize) {
        self.reveal(i);
        self.cursor = i;
        self.follow = true;
    }

    /// Opens the folders that row `i` is in.
    fn reveal(&mut self, i: usize) {
        let row = &self.rows[i];
        self.collapsed.retain(|c| !row.inside(c));
    }

    /// Puts the cursor on the row for `path`, a file or a folder, if it's listed. The folders
    /// it's in open.
    pub fn select(&mut self, path: &str) {
        if let Some(i) = self.rows.iter().position(|r| r.path == path) {
            self.go(i);
        }
    }

    /// `j`/`k`: to the next or previous visible row.
    pub fn step(&mut self, down: bool) {
        let visible = self.visible();
        let at = visible.iter().position(|&i| i == self.cursor).unwrap_or(0);
        let next = if down {
            visible.get(at + 1)
        } else {
            at.checked_sub(1).and_then(|a| visible.get(a))
        };
        if let Some(&i) = next {
            self.go(i);
        }
    }

    /// Opens or closes the folder at row `i`.
    fn toggle(&mut self, i: usize) {
        let path = &self.rows[i].path;
        if !self.collapsed.remove(path) {
            self.collapsed.insert(path.clone());
        }
    }

    /// `l` on a folder: open it, or if it's already open, go to its first row.
    fn right(&mut self) {
        let path = &self.rows[self.cursor].path;
        if !self.collapsed.remove(path) {
            self.step(true);
        }
    }

    /// `h`: close the folder under the cursor. On a file or a closed folder, go to the folder
    /// it's in instead.
    fn left(&mut self) {
        let row = &self.rows[self.cursor];
        if row.is_folder() && self.collapsed.insert(row.path.clone()) {
            return;
        }
        // Rows come in tree order, so the parent is the nearest folder above, one level up.
        let depth = row.depth;
        if let Some(i) = self.rows[..self.cursor]
            .iter()
            .rposition(|r| r.is_folder() && r.depth + 1 == depth)
        {
            self.go(i);
        }
    }

    pub fn origin(&self) -> Origin {
        Origin {
            cursor: self.cursor,
            top: self.top,
            collapsed: self.collapsed.clone(),
        }
    }

    pub fn restore(&mut self, o: Origin) {
        (self.cursor, self.top, self.collapsed) = (o.cursor, o.top, o.collapsed);
    }

    /// Moves the cursor to the next row whose name matches, or the previous one, wrapping at
    /// the ends. Rows inside closed folders count, and the folders holding the match open.
    /// Only the name is matched, so `src` finds the `src/` folder and not the files in it.
    /// Returns whether anything matched.
    pub fn search(&mut self, regex: &regex::Regex, forward: bool) -> bool {
        let n = self.rows.len();
        let found = (1..=n)
            .map(|k| {
                if forward {
                    (self.cursor + k) % n
                } else {
                    (self.cursor + n - k) % n
                }
            })
            .find(|&i| regex.is_match(&self.rows[i].name));
        if let Some(i) = found {
            self.go(i);
        }
        found.is_some()
    }

    pub fn selected(&self) -> Option<&str> {
        self.rows.get(self.cursor)?.file()
    }

    /// The row on the screen row `y` of `area`, as last drawn.
    pub fn row_at(&self, area: Rect, y: u16) -> Option<usize> {
        let i = self.top + y.checked_sub(area.y + 1)? as usize;
        self.visible().get(i).copied()
    }

    fn render_in(&mut self, buf: &mut Buffer, area: Rect, current: &str, theme: &Theme) {
        let base = theme.get("ui.background");
        buf.set_style(area, base);
        let title = theme.get("ui.text").patch(ratatui::style::Style::default());
        buf.set_stringn(area.x, area.y, " Changed files", area.width as usize, title);
        let height = area.height.saturating_sub(1) as usize;
        let visible = self.visible();
        if std::mem::take(&mut self.follow) {
            let at = visible.iter().position(|&i| i == self.cursor).unwrap_or(0);
            if at < self.top {
                self.top = at;
            } else if height > 0 && at >= self.top + height {
                self.top = at + 1 - height;
            }
        }
        self.top = self.top.min(visible.len().saturating_sub(height));
        let w = area.width.saturating_sub(1) as usize; // the last column separates it
        for (row, &i) in visible.iter().enumerate().skip(self.top).take(height) {
            let r = &self.rows[i];
            let y = area.y + 1 + (row - self.top) as u16;
            let mut style = theme.get(if !r.is_folder() {
                "ui.text"
            } else {
                "ui.text.directory"
            });
            if r.file() == Some(current) {
                style = style.patch(theme.get("ui.text.focus"));
            }
            if i == self.cursor && self.focused {
                style = style.patch(theme.get("ui.menu.selected"));
                buf.set_style(Rect::new(area.x, y, w as u16, 1), style);
            }
            // Right-aligned: +added -removed and a thread badge.
            let mut tail = vec![];
            if r.added > 0 {
                tail.push((format!("+{}", r.added), theme.get("diff.plus")));
            }
            if r.removed > 0 {
                tail.push((format!("-{}", r.removed), theme.get("diff.minus")));
            }
            if r.threads > 0 {
                tail.push((format!("●{}", r.threads), theme.get("info")));
            }
            let tail_w: usize = tail.iter().map(|(t, _)| t.width() + 1).sum();
            let marker = match (r.is_folder(), self.collapsed.contains(&r.path)) {
                (false, _) => "",
                (true, false) => "▾ ",
                (true, true) => "▸ ",
            };
            let name = format!("{}{marker}{}", "  ".repeat(r.depth), clean(&r.name));
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
    type Cmd = TreeCmd;

    /// Runs one of the tree's commands. Opening a file gives the keys back to it.
    fn command(&mut self, cmd: TreeCmd, _cx: &Cx) -> Outcome {
        let folder = self.rows.get(self.cursor).is_some_and(Row::is_folder);
        match cmd {
            TreeCmd::Next => self.step(true),
            TreeCmd::Prev => self.step(false),
            TreeCmd::Open if folder => self.toggle(self.cursor),
            TreeCmd::OpenOrInto if folder => self.right(),
            TreeCmd::CloseOrUp => self.left(),
            TreeCmd::Open | TreeCmd::OpenOrInto => {
                if let Some(p) = self.selected().map(str::to_string) {
                    self.focused = false;
                    return Outcome::Msg(Msg::Open(Target::Path(p)));
                }
            }
            TreeCmd::Unfocus => self.focused = false,
            TreeCmd::Close => return Outcome::Msg(Msg::CloseTree),
            TreeCmd::Search(forward) => return Outcome::Msg(Msg::TreeSearch(forward)),
            TreeCmd::SearchNext(forward) => return Outcome::Msg(Msg::TreeSearchNext(forward)),
        }
        Outcome::Consumed
    }

    /// A click anywhere in the tree focuses it. A click on a file also opens that file, and a
    /// click on a folder opens or closes it. The wheel scrolls the rows without moving the
    /// cursor.
    fn mouse(&mut self, m: MouseEvent, area: Rect, _cx: &Cx) -> Outcome {
        if !area.contains((m.column, m.row).into()) {
            return Outcome::Ignored;
        }
        match m.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                self.focused = true;
                if let Some(i) = self.row_at(area, m.row) {
                    self.go(i);
                    match self.rows[i].file() {
                        Some(p) => return Outcome::Msg(Msg::Open(Target::Path(p.to_string()))),
                        None => self.toggle(i),
                    }
                }
            }
            MouseEventKind::ScrollDown => {
                let height = area.height.saturating_sub(1) as usize;
                self.top = (self.top + 3).min(self.visible().len().saturating_sub(height));
            }
            MouseEventKind::ScrollUp => self.top = self.top.saturating_sub(3),
            _ => {}
        }
        Outcome::Consumed
    }

    /// Draws the tree in `area`, the file on screen marked. When that file has changed since
    /// the last draw, the folders holding it open. If the tree isn't focused, its cursor moves
    /// to the file too. Doing this here covers every way of changing the file.
    fn render(&mut self, buf: &mut Buffer, area: Rect, cx: &Cx) {
        let current = cx.state.map_or("", |s| s.main_path());
        if current != self.shown {
            self.shown = current.to_string();
            match self.rows.iter().position(|r| r.path == current) {
                Some(i) if self.focused => self.reveal(i),
                Some(i) => self.go(i),
                None => {}
            }
        }
        self.render_in(buf, area, current, cx.theme);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::keymap::{Action, Context, Key, Keymaps, Node};

    /// Presses `key` in the tree, through the tree's keymap. An unbound key does nothing.
    fn press(t: &mut Tree, key: &str, cx: &Cx) -> Outcome {
        let keymaps = Keymaps::default();
        match keymaps.get(Context::Tree).get(Key::parse(key).unwrap()) {
            Some(Node::Cmd(c)) => match c.action {
                Action::Tree(cmd) => t.command(cmd, cx),
                _ => panic!("{} isn't a tree command", c.name),
            },
            _ => Outcome::Consumed,
        }
    }

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

    /// The path under the cursor, a folder's included.
    fn at(t: &Tree) -> &str {
        &t.rows[t.cursor].path
    }

    fn shown(t: &Tree) -> Vec<&str> {
        t.visible()
            .iter()
            .map(|&i| t.rows[i].path.as_str())
            .collect()
    }

    #[test]
    fn moving_lands_on_folders_and_stops_at_the_ends() {
        let mut t = Tree::new(vec![e("a/x", 0, 0, 0), e("b/y", 0, 0, 0)]);
        assert_eq!(
            t.selected(),
            Some("a/x"),
            "starts on the first file, not a folder"
        );
        t.step(true);
        assert_eq!(at(&t), "b/");
        assert_eq!(t.selected(), None);
        t.step(true);
        t.step(true);
        assert_eq!(at(&t), "b/y");
        t.step(false);
        t.step(false);
        t.step(false);
        t.step(false);
        assert_eq!(at(&t), "a/");
        t.select("b/y");
        assert_eq!(t.selected(), Some("b/y"));
        t.select("nope");
        assert_eq!(t.selected(), Some("b/y"));
        assert!(Tree::new(vec![]).selected().is_none());
    }

    fn nested() -> Tree {
        Tree::new(vec![
            e("README.md", 1, 0, 0),
            e("src/main.rs", 2, 1, 1),
            e("src/ui/mod.rs", 3, 0, 2),
            e("src/ui/view.rs", 4, 2, 0),
            e("tests/mcp.rs", 5, 0, 0),
        ])
    }

    #[test]
    fn a_closed_folder_hides_everything_under_it() {
        let mut t = nested();
        t.collapsed.insert("src/".into());
        assert_eq!(shown(&t), ["README.md", "src/", "tests/", "tests/mcp.rs"]);
        // Opening it again shows the folder inside it as it was left.
        t.collapsed.insert("src/ui/".into());
        t.collapsed.remove("src/");
        assert_eq!(
            shown(&t),
            [
                "README.md",
                "src/",
                "src/main.rs",
                "src/ui/",
                "tests/",
                "tests/mcp.rs"
            ]
        );
        let src = &t.rows[1];
        assert_eq!((src.added, src.removed, src.threads), (9, 3, 3), "totals");
        let ui = &t.rows[3];
        assert_eq!((ui.added, ui.removed, ui.threads), (7, 2, 2));
    }

    #[test]
    fn a_rebuild_keeps_closed_folders_and_the_cursor() {
        let mut t = nested();
        t.collapsed
            .extend(["src/ui/".to_string(), "tests/".to_string()]);
        t.select("src/");
        t.focused = false;
        // tests/ is gone from the new tree, so it's no longer closed.
        let t = t.rebuilt(vec![
            e("src/main.rs", 0, 0, 0),
            e("src/ui/mod.rs", 0, 0, 0),
            e("z.rs", 0, 0, 0),
        ]);
        assert_eq!(t.collapsed, HashSet::from(["src/ui/".to_string()]));
        assert_eq!(at(&t), "src/");
        assert!(!t.focused);
    }

    #[test]
    fn moving_skips_what_a_closed_folder_hides() {
        let mut t = nested();
        t.collapsed.insert("src/".into());
        t.select("README.md");
        t.step(true);
        t.step(true);
        assert_eq!(at(&t), "tests/");
        t.step(false);
        assert_eq!(at(&t), "src/");
    }

    #[test]
    fn h_and_l_open_close_and_move() {
        let cx_theme = Theme::load("default", crate::theme::ColorSupport::TrueColor).unwrap();
        let cx = Cx {
            screen: Rect::new(0, 0, 80, 24),
            theme: &cx_theme,
            state: None,
        };
        let mut t = nested();
        // At the top: h on a top-level file has nowhere to go.
        t.select("README.md");
        press(&mut t, "h", &cx);
        assert_eq!(at(&t), "README.md");
        // In the middle: h on a file goes to its folder, then closes it, then goes up.
        t.select("src/ui/view.rs");
        press(&mut t, "h", &cx);
        assert_eq!(at(&t), "src/ui/");
        press(&mut t, "h", &cx);
        assert!(t.collapsed.contains("src/ui/"));
        press(&mut t, "h", &cx);
        assert_eq!(at(&t), "src/");
        // l opens a closed folder, then goes to its first row.
        t.select("src/ui/");
        press(&mut t, "l", &cx);
        assert!(!t.collapsed.contains("src/ui/"));
        assert_eq!(at(&t), "src/ui/");
        press(&mut t, "l", &cx);
        assert_eq!(at(&t), "src/ui/mod.rs");
        // l on a file opens it.
        assert_eq!(
            press(&mut t, "l", &cx),
            Outcome::Msg(Msg::Open(Target::Path("src/ui/mod.rs".into())))
        );
        // At the bottom: ret toggles a folder, and h on a top-level closed folder stays.
        t.select("tests/");
        assert_eq!(press(&mut t, "ret", &cx), Outcome::Consumed);
        assert!(t.collapsed.contains("tests/"));
        press(&mut t, "h", &cx);
        assert_eq!(at(&t), "tests/");
        press(&mut t, "ret", &cx);
        assert!(!t.collapsed.contains("tests/"));
        press(&mut t, "j", &cx);
        assert_eq!(at(&t), "tests/mcp.rs");
        press(&mut t, "j", &cx);
        assert_eq!(at(&t), "tests/mcp.rs", "the last row");
    }

    #[test]
    fn rows_show_counts_and_badges() {
        let theme = Theme::load("default", crate::theme::ColorSupport::TrueColor).unwrap();
        let mut t = Tree::new(vec![e("src/a.rs", 12, 3, 2), e("b.rs", 0, 0, 0)]);
        let area = Rect::new(0, 0, 30, 5);
        let mut buf = Buffer::empty(area);
        t.render_in(&mut buf, area, "b.rs", &theme);
        let row = |buf: &Buffer, y| {
            (0..30)
                .map(|x| buf[(x, y)].symbol())
                .collect::<String>()
                .trim_end()
                .to_string()
        };
        assert_eq!(
            row(&buf, 0).trim_end_matches('│').trim_end(),
            " Changed files"
        );
        assert!(row(&buf, 1).starts_with(" b.rs"));
        assert!(
            row(&buf, 3).contains("a.rs") && row(&buf, 3).contains("+12 -3 ●2"),
            "{}",
            row(&buf, 3)
        );
        assert!(
            row(&buf, 2).contains("▾ src/") && row(&buf, 2).contains("+12 -3 ●2"),
            "{}",
            row(&buf, 2)
        );
        assert_eq!(t.row_at(area, 3), Some(2));
        assert_eq!(t.row_at(area, 2), Some(1), "a folder");
        assert_eq!(t.row_at(area, 0), None, "the title");
        t.collapsed.insert("src/".into());
        let mut buf = Buffer::empty(area);
        t.render_in(&mut buf, area, "b.rs", &theme);
        assert!(
            row(&buf, 2).contains("▸ src/") && row(&buf, 2).contains("+12 -3 ●2"),
            "{}",
            row(&buf, 2)
        );
        assert!(!row(&buf, 3).contains("a.rs"), "{}", row(&buf, 3));
        // Selecting a row opens the folders it's in.
        t.select("src/a.rs");
        assert!(t.collapsed.is_empty());
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
        let mut t = Tree::new(vec![e("a.rs", 1, 0, 0), e("src/b.rs", 1, 0, 0)]);
        assert_eq!(press(&mut t, "j", &cx), Outcome::Consumed);
        assert_eq!(at(&t), "src/", "j lands on a folder");
        press(&mut t, "j", &cx);
        assert_eq!(t.selected(), Some("src/b.rs"));
        assert_eq!(
            press(&mut t, "x", &cx),
            Outcome::Consumed,
            "every key is the tree's"
        );
        assert_eq!(
            press(&mut t, "l", &cx),
            Outcome::Msg(Msg::Open(Target::Path("src/b.rs".into())))
        );
        assert!(!t.focused, "opening a file gives the keys back to it");
        assert_eq!(press(&mut t, "q", &cx), Outcome::Msg(Msg::CloseTree));

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
        assert!(t.collapsed.contains("src/"), "a click closes a folder");
        assert_eq!(at(&t), "src/");
        t.mouse(click(3, 2), area, &cx);
        assert!(t.collapsed.is_empty(), "and opens it again");
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
