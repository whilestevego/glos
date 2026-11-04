//! The reader: the full-file view with Helix's gutter, the statusline, the command line and the
//! key help panel.

mod commands;
mod keymap;

use std::{borrow::Cow, num::NonZeroUsize};

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use helix_core::{Rope, RopeSlice, Selection, doc_formatter::TextFormat};
use helix_stdx::rope::RopeSliceExt;
use ratatui::{
    DefaultTerminal, Frame,
    buffer::Buffer,
    layout::Rect,
    style::Style,
    widgets::{Block, Borders, Clear},
};
use unicode_width::UnicodeWidthStr;

use crate::{
    diff::{self, Hunk},
    source::{self, Content, Kind, Mode as FileMode, Source},
    syntax::Loader,
    theme::{ColorSupport, Theme},
};
use commands::Ctx;
use keymap::{Action, AppAction, Key, Keymaps, Node};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Normal,
    Select,
}

/// A key that's waiting for one more character (`f`, `mi`, …).
#[derive(Clone, Copy)]
enum NextKey {
    Char(fn(&mut Ctx, char), &'static str),
    Textobject(bool),
}

/// One changed file, loaded the first time it's shown.
struct File {
    text: Rope,
    hunks: Vec<Hunk>,
    sel: Selection,
    top: usize,
    left: usize,
    syntax: Option<tree_house::Syntax>,
    tab_width: usize,
    /// One-line headers above the text: the kind of change, binary, collapsed…
    notices: Vec<String>,
    /// Lockfiles and generated files start collapsed; `enter` expands them.
    collapsed: bool,
    /// Binary files have no text to show.
    binary: bool,
}

pub struct App {
    source: Source,
    files: Vec<Option<File>>,
    cur: usize,
    mode: Mode,
    theme: Theme,
    loader: Loader,
    keymaps: Keymaps,
    /// Keys typed so far into a menu (`g`, `]`…), shown at the bottom right.
    pending: Vec<Key>,
    count: Option<NonZeroUsize>,
    next_key: Option<NextKey>,
    /// The `:` command line, while it's open.
    prompt: Option<String>,
    status: Option<(String, bool)>,
    /// The text area from the last draw, for commands that page or align the view.
    view: Rect,
    quit: bool,
}

pub fn run(source: Source, theme: &str) -> Result<()> {
    let theme = Theme::load(theme, ColorSupport::detect())?;
    let loader = Loader::new(&theme)?;
    let mut app = App {
        files: (0..source.changes.len()).map(|_| None).collect(),
        source,
        cur: 0,
        mode: Mode::Normal,
        theme,
        loader,
        keymaps: Keymaps::default(),
        pending: vec![],
        count: None,
        next_key: None,
        prompt: None,
        status: None,
        view: Rect::default(),
        quit: false,
    };
    // Raw mode and the alternate screen; the terminal is restored on a panic too.
    let mut terminal = ratatui::init();
    let result = app.event_loop(&mut terminal);
    ratatui::restore();
    result
}

impl App {
    fn event_loop(&mut self, terminal: &mut DefaultTerminal) -> Result<()> {
        let size = terminal.size()?;
        self.view = Rect::new(0, 0, size.width, size.height.saturating_sub(2));
        self.load(self.cur);
        while !self.quit {
            terminal.draw(|f| self.render(f))?;
            match event::read()? {
                Event::Key(k) if k.kind != KeyEventKind::Release => {
                    if self.key(Key::from(k)) == Some(AppAction::Suspend) {
                        suspend(terminal)?;
                    }
                }
                _ => {} // resizes and anything else just redraw
            }
        }
        Ok(())
    }

    fn load(&mut self, i: usize) {
        if self.files[i].is_some() {
            return;
        }
        let change = &self.source.changes[i];
        let read = |c: &Content| self.source.read(c).ok().flatten().unwrap_or_default();
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
        let collapsed = self.source.collapsed(change);
        if collapsed {
            notices.push("generated file, collapsed; enter expands".to_string());
        }
        let text = if binary { Rope::new() } else { Rope::from(String::from_utf8_lossy(shown).as_ref()) };
        let hunks = if binary || change.new == Content::None {
            vec![]
        } else {
            diff::hunks(&String::from_utf8_lossy(&old), &String::from_utf8_lossy(&new))
        };
        let path = self.source.root.join(&change.path);
        let lang = self.loader.language_for(&path, text.slice(..));
        let syntax = lang.and_then(|l| self.loader.parse(text.slice(..), l));
        let mut file = File {
            tab_width: self.loader.tab_width(lang),
            text,
            hunks,
            sel: Selection::point(0),
            top: 0,
            left: 0,
            syntax,
            notices,
            collapsed,
            binary,
        };
        // Open on the first change, as a reviewer wants.
        if let Some(h) = file.hunks.first() {
            let line = (h.after.start as usize).min(file.text.len_lines().saturating_sub(1));
            file.sel = Selection::point(file.text.line_to_char(line));
            file.top = line.saturating_sub(commands::SCROLLOFF);
        }
        self.files[i] = Some(file);
    }

    fn file(&self) -> &File {
        self.files[self.cur].as_ref().expect("the current file is loaded")
    }

    fn file_mut(&mut self) -> &mut File {
        self.files[self.cur].as_mut().expect("the current file is loaded")
    }

    /// Rows above the text taken by the file's notices.
    fn header_rows(&self) -> u16 {
        self.file().notices.len() as u16
    }

    fn text_height(&self) -> usize {
        self.view.height.saturating_sub(self.header_rows()) as usize
    }

    /// Handles one key. Returns the app action that needs the terminal (suspend), if any.
    fn key(&mut self, key: Key) -> Option<AppAction> {
        self.status = None;
        if let Some(prompt) = &mut self.prompt {
            match key.code {
                KeyCode::Esc => self.prompt = None,
                KeyCode::Enter => {
                    let line = self.prompt.take().unwrap_or_default();
                    self.typed_command(line.trim());
                }
                KeyCode::Backspace => {
                    if prompt.pop().is_none() {
                        self.prompt = None;
                    }
                }
                KeyCode::Char(c) if key.char().is_some() || key.mods == crossterm::event::KeyModifiers::NONE => {
                    prompt.push(c)
                }
                _ => {}
            }
            return None;
        }
        if let Some(next) = self.next_key.take() {
            self.pending.clear();
            if let Some(ch) = key.char().or(match key.code {
                KeyCode::Enter => Some('\n'),
                KeyCode::Tab => Some('\t'),
                _ => None,
            }) {
                match next {
                    NextKey::Char(f, _) => self.doc_command(|cx| f(cx, ch)),
                    NextKey::Textobject(around) => self.doc_command(|cx| commands::select_textobject(cx, around, ch)),
                }
            }
            self.count = None;
            return None;
        }
        // A count, as in Helix: digits before a command (a leading 0 isn't one).
        if self.pending.is_empty() {
            if let Some(d) = key.char().and_then(|c| c.to_digit(10)) {
                if d != 0 || self.count.is_some() {
                    let n = self.count.map_or(0, NonZeroUsize::get) * 10 + d as usize;
                    self.count = NonZeroUsize::new(n.min(100_000_000));
                    return None;
                }
            }
        }
        let keymap = if self.mode == Mode::Select { &self.keymaps.select } else { &self.keymaps.normal };
        let mut node = None;
        let mut menu = keymap;
        for k in self.pending.iter().chain(std::iter::once(&key)) {
            match menu.get(*k) {
                Some(Node::Menu(m)) => {
                    menu = m;
                    node = None;
                }
                Some(Node::Cmd(c)) => node = Some(*c),
                None => {
                    // Unbound: drop the sequence, as Helix does.
                    self.pending.clear();
                    self.count = None;
                    return None;
                }
            }
        }
        let Some(command) = node else {
            self.pending.push(key); // into a menu; the help panel opens
            return None;
        };
        self.pending.clear();
        let result = match command.action {
            Action::Doc(f) => {
                self.doc_command(f);
                None
            }
            Action::NextChar(f) => {
                self.next_key = Some(NextKey::Char(f, command.doc));
                return None; // keep the count for when the character comes
            }
            Action::Textobject(around) => {
                self.next_key = Some(NextKey::Textobject(around));
                return None;
            }
            Action::App(a) => self.app_command(a),
        };
        self.count = None;
        result
    }

    fn app_command(&mut self, action: AppAction) -> Option<AppAction> {
        match action {
            AppAction::SelectMode => self.mode = Mode::Select,
            AppAction::NormalMode => self.mode = Mode::Normal,
            AppAction::CommandMode => self.prompt = Some(String::new()),
            AppAction::NextFile | AppAction::PrevFile => {
                let n = self.files.len();
                let step = self.count.map_or(1, NonZeroUsize::get) % n;
                self.cur = if action == AppAction::NextFile { (self.cur + step) % n } else { (self.cur + n - step) % n };
                self.load(self.cur);
            }
            AppAction::Suspend => return Some(AppAction::Suspend),
        }
        None
    }

    /// `:` commands. ponytail: a few for now; the full set with completion comes with M2.
    fn typed_command(&mut self, line: &str) {
        let (name, _args) = line.split_once(' ').unwrap_or((line, ""));
        match name {
            "" => {}
            "q" | "quit" | "q!" | "quit!" | "qa" | "qa!" => self.quit = true,
            n if n.parse::<usize>().is_ok() => {
                self.count = NonZeroUsize::new(n.parse().unwrap());
                self.doc_command(commands::goto_line);
                self.count = None;
            }
            other => self.status = Some((format!("no such command: '{other}'"), true)),
        }
    }

    /// Runs a command on the current file's selection, then keeps the cursor in view.
    fn doc_command(&mut self, f: impl FnOnce(&mut Ctx)) {
        let (count, extend, height, width) = (self.count, self.mode == Mode::Select, self.text_height(), self.view.width);
        let file = self.files[self.cur].as_mut().expect("loaded");
        if file.collapsed || file.binary {
            return;
        }
        let mut cx = Ctx {
            text: &file.text,
            sel: file.sel.clone(),
            count,
            extend,
            hunks: &file.hunks,
            top: file.top,
            height,
            fmt: TextFormat { tab_width: file.tab_width as u16, viewport_width: width, ..TextFormat::default() },
            status: None,
        };
        f(&mut cx);
        let (sel, top, status) = (cx.sel.ensure_invariants(file.text.slice(..)), cx.top, cx.status);
        file.sel = sel;
        file.top = top;
        if let Some(s) = status {
            self.status = Some((s, false));
        }
        self.scroll_to_cursor();
    }

    /// Keeps the primary cursor `scrolloff` lines and columns inside the view.
    fn scroll_to_cursor(&mut self) {
        let height = self.text_height();
        let width = (self.view.width as usize).saturating_sub(self.gutter_width());
        let file = self.file_mut();
        let text = file.text.slice(..);
        let cursor = file.sel.primary().cursor(text);
        let line = text.char_to_line(cursor);
        let so = commands::SCROLLOFF.min(height.saturating_sub(1) / 2);
        if line < file.top + so {
            file.top = line.saturating_sub(so);
        } else if height > 0 && line + so >= file.top + height {
            file.top = line + so + 1 - height;
        }
        let col = visual_col(text, cursor, file.tab_width);
        let so = commands::SCROLLOFF.min(width.saturating_sub(1) / 2);
        if col < file.left + so {
            file.left = col.saturating_sub(so);
        } else if width > 0 && col + so >= file.left + width {
            file.left = col + so + 1 - width;
        }
    }

    /// Helix's default gutters: a diagnostics column (glos's thread markers), a spacer, line
    /// numbers (at least 3 wide), a spacer and the diff signs.
    fn gutter_width(&self) -> usize {
        let lines = self.file().text.len_lines();
        let digits = lines.to_string().len().max(3);
        1 + 1 + digits + 1 + 1
    }

    fn render(&mut self, f: &mut Frame) {
        let area = f.area();
        let buf = f.buffer_mut();
        buf.set_style(area, self.theme.get("ui.background"));
        if area.height < 3 {
            return;
        }
        self.view = Rect::new(0, 0, area.width, area.height - 2);
        self.render_file(buf);
        self.render_statusline(buf, Rect::new(0, area.height - 2, area.width, 1));
        self.render_command_line(buf, Rect::new(0, area.height - 1, area.width, 1));
        self.render_help(f, area);
    }

    fn render_file(&self, buf: &mut Buffer) {
        let theme = &self.theme;
        let file = self.file();
        let view = self.view;
        let mut y = view.y;
        for notice in &file.notices {
            buf.set_stringn(view.x + 1, y, clean(notice), view.width as usize, theme.get("ui.virtual"));
            y += 1;
        }
        if file.collapsed || file.binary {
            return;
        }
        let text = file.text.slice(..);
        let height = view.height.saturating_sub(y - view.y) as usize;
        let last = (file.top + height).min(text.len_lines());

        let start_byte = text.line_to_byte(file.top.min(text.len_lines()));
        let end_byte = text.line_to_byte(last);
        let spans = match &file.syntax {
            Some(s) => self.loader.highlights(s, text, start_byte..end_byte, theme),
            None => vec![],
        };
        let mut span = 0;

        let digits = text.len_lines().to_string().len().max(3);
        let gutter = self.gutter_width() as u16;
        let cursor_lines: Vec<usize> = file.sel.iter().map(|r| text.char_to_line(r.cursor(text))).collect();
        let primary = file.sel.primary_index();
        let ranges: Vec<_> = file.sel.iter().map(|r| (r.from(), r.to(), r.cursor(text))).collect();

        let text_style = theme.get("ui.text");
        let (sel_style, sel_primary, cursor, cursor_primary) = self.selection_styles();
        let (added, deleted, modified) =
            (theme.get("diff.plus.gutter"), theme.get("diff.minus.gutter"), theme.get("diff.delta.gutter"));

        let mut hunk_i = 0;
        for line in file.top..last {
            // Gutter: line number, then the diff sign as Helix's diff gutter draws it.
            let selected = cursor_lines.contains(&line);
            let nr_style = theme.get(if selected { "ui.linenr.selected" } else { "ui.linenr" });
            buf.set_string(view.x + 2, y, format!("{:>digits$}", line + 1), nr_style);
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
                buf.set_string(view.x + gutter - 1, y, sign, style);
            }

            // Text, from the horizontal offset.
            let mut col = 0usize;
            let mut ch = text.line_to_char(line);
            let x0 = view.x + gutter;
            let width = view.width.saturating_sub(gutter) as usize;
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
                    let w = file.tab_width - col % file.tab_width;
                    (" ".repeat(w).into(), w)
                } else {
                    let s = clean(&g);
                    let w = s.width().max(1);
                    (s, w)
                };
                if col + w > file.left && col < file.left + width {
                    if col >= file.left && col + w <= file.left + width {
                        let x = x0 + (col - file.left) as u16;
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
                }
                col += w;
                ch += g.chars().count();
                if newline {
                    break;
                }
            }
            // The cursor can sit at the very end of a file with no trailing newline.
            if ch == text.len_chars() && ranges.iter().any(|r| r.2 == ch) && col >= file.left && col < file.left + width {
                buf[(x0 + (col - file.left) as u16, y)].set_symbol(" ").set_style(cursor_primary);
            }
            y += 1;
        }
    }

    /// (selection, primary selection, cursor, primary cursor), as Helix picks them per mode.
    fn selection_styles(&self) -> (Style, Style, Style, Style) {
        let t = &self.theme;
        let sel = t.get("ui.selection");
        let sel_primary = t.try_get("ui.selection.primary").unwrap_or(sel);
        let base_cursor = t.try_get_exact("ui.cursor").unwrap_or(sel);
        let base_primary = t.try_get("ui.cursor.primary").unwrap_or(base_cursor);
        let mode = if self.mode == Mode::Select { "select" } else { "normal" };
        let cursor = t.try_get_exact(&format!("ui.cursor.{mode}")).unwrap_or(base_cursor);
        let primary = t.try_get_exact(&format!("ui.cursor.primary.{mode}")).unwrap_or(base_primary);
        (sel, sel_primary, cursor, primary)
    }

    fn render_statusline(&self, buf: &mut Buffer, area: Rect) {
        let t = &self.theme;
        let base = t.get("ui.statusline");
        buf.set_style(area, base);
        let file = self.file();
        let change = &self.source.changes[self.cur];
        let mode = if self.mode == Mode::Select { " SEL " } else { " NOR " };
        let left = format!("{mode} {}", clean(&change.path));
        buf.set_stringn(area.x, area.y, &left, area.width as usize, base);

        let text = file.text.slice(..);
        let cursor = file.sel.primary().cursor(text);
        let line = text.char_to_line(cursor);
        let col = cursor - text.line_to_char(line);
        let sels = file.sel.len();
        let right = format!(
            " {}/{} files  {} sel{}  {}:{} ",
            self.cur + 1,
            self.files.len(),
            sels,
            if sels == 1 { "" } else { "s" },
            line + 1,
            col + 1
        );
        let w = right.width() as u16;
        if w + left.width() as u16 + 1 < area.width {
            buf.set_string(area.x + area.width - w, area.y, &right, base);
        }
    }

    fn render_command_line(&self, buf: &mut Buffer, area: Rect) {
        let t = &self.theme;
        if let Some(p) = &self.prompt {
            buf.set_stringn(area.x, area.y, format!(":{}", clean(p)), area.width as usize, t.get("ui.text"));
            let x = area.x + 1 + p.width() as u16;
            if x < area.x + area.width {
                buf[(x, area.y)].set_style(t.get("ui.cursor.primary"));
            }
            return;
        }
        if let Some((msg, error)) = &self.status {
            let style = if *error { t.get("error") } else { t.get("ui.text") };
            buf.set_stringn(area.x, area.y, clean(msg), area.width as usize, style);
        }
        // Pending keys and count at the bottom right, as Helix shows them.
        let mut pending: String = self.count.map(|c| c.to_string()).unwrap_or_default();
        for k in &self.pending {
            pending += &k.to_string();
        }
        if !pending.is_empty() {
            let w = pending.width() as u16;
            buf.set_string(area.x + area.width.saturating_sub(w + 1), area.y, &pending, t.get("ui.text"));
        }
    }

    /// The key help panel (Helix's infobox): what can follow the keys typed so far.
    fn render_help(&self, f: &mut Frame, area: Rect) {
        let (title, rows): (&str, Vec<(String, &str)>) = if let Some(next) = self.next_key {
            match next {
                NextKey::Textobject(around) => (
                    if around { "Match around" } else { "Match inside" },
                    commands::TEXTOBJECT_HELP.iter().map(|(k, d)| (k.to_string(), *d)).collect(),
                ),
                NextKey::Char(_, doc) => (doc, vec![]),
            }
        } else if !self.pending.is_empty() {
            let keymap = if self.mode == Mode::Select { &self.keymaps.select } else { &self.keymaps.normal };
            let mut menu = keymap;
            for k in &self.pending {
                if let Some(Node::Menu(m)) = menu.get(*k) {
                    menu = m;
                }
            }
            (menu.name, menu.help())
        } else {
            return;
        };
        if rows.is_empty() {
            return;
        }
        let key_w = rows.iter().map(|(k, _)| k.width()).max().unwrap_or(0);
        let lines: Vec<String> = rows.iter().map(|(k, d)| format!("{k:key_w$}  {d}")).collect();
        let width = (lines.iter().map(|l| l.width()).max().unwrap_or(0).max(title.width()) + 4) as u16;
        let height = lines.len() as u16 + 2;
        let width = width.min(area.width);
        let height = height.min(area.height.saturating_sub(2));
        let rect = Rect::new(area.width - width, area.height.saturating_sub(2 + height), width, height);
        let t = &self.theme;
        let popup = t.get("ui.popup.info");
        f.render_widget(Clear, rect);
        f.render_widget(
            Block::default().borders(Borders::ALL).title(title).style(popup).border_style(popup),
            rect,
        );
        let text_style = popup.patch(t.get("ui.text.info"));
        for (i, l) in lines.iter().enumerate().take(height.saturating_sub(2) as usize) {
            f.buffer_mut().set_stringn(rect.x + 2, rect.y + 1 + i as u16, l, width.saturating_sub(3) as usize, text_style);
        }
    }
}

/// The display column of char `pos` on its line, with tabs expanded.
fn visual_col(text: RopeSlice, pos: usize, tab_width: usize) -> usize {
    let line = text.char_to_line(pos);
    let start = text.line_to_char(line);
    let mut col = 0;
    for g in text.slice(start..pos).graphemes() {
        let g: Cow<str> = g.into();
        col += if g == "\t" { tab_width - col % tab_width } else { clean(&g).width().max(1) };
    }
    col
}

/// Untrusted text is never drawn raw: control characters become visible symbols, so escape
/// sequences in code or agent comments can't reach the terminal.
fn clean(s: &str) -> Cow<'_, str> {
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

/// `ctrl-z`: give the terminal back, stop, and take it again on `fg`.
fn suspend(terminal: &mut DefaultTerminal) -> Result<()> {
    ratatui::restore();
    // SAFETY: raising a signal on ourselves; the shell resumes us with SIGCONT.
    unsafe { libc::raise(libc::SIGTSTP) };
    crossterm::terminal::enable_raw_mode()?;
    crossterm::execute!(std::io::stdout(), crossterm::terminal::EnterAlternateScreen)?;
    terminal.clear()?;
    Ok(())
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
        assert_eq!(visual_col(t.slice(..), 5, 4), 8 + 1);
    }
}
