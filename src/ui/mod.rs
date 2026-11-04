//! The reader: the full-file view, the statusline, the command line, the key help panel and the
//! pickers.

mod commands;
mod keymap;
mod picker;
mod view;

use std::{collections::HashMap, num::NonZeroUsize};

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use helix_core::{Selection, doc_formatter::TextFormat};
use ratatui::{
    DefaultTerminal, Frame,
    buffer::Buffer,
    layout::Rect,
    style::Style,
    widgets::{Block, Borders, Clear},
};
use unicode_width::UnicodeWidthStr;

use crate::{
    clipboard,
    config::Config,
    source::{Kind, Source},
    syntax::Loader,
    theme::{ColorSupport, Theme},
};
use commands::Ctx;
use keymap::{Action, AppAction, Key, Keymaps, Node};
use picker::{Item, Outcome, Picker, Target};
use view::{File, clean};

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

/// The file on screen: one of the review's changes, or another file opened for context.
#[derive(Clone, PartialEq, Eq)]
enum Cur {
    Change(usize),
    Context(String),
}

enum PromptKind {
    Command,
    /// A search, forward or back; it previews as you type and `esc` puts the selection back.
    Search { forward: bool, origin: Selection, top: usize },
}

struct Prompt {
    kind: PromptKind,
    line: String,
}

pub struct App {
    source: Source,
    files: Vec<Option<File>>,
    context: HashMap<String, File>,
    cur: Cur,
    mode: Mode,
    theme: Theme,
    loader: Loader,
    keymaps: Keymaps,
    /// Keys typed so far into a menu (`g`, `]`…), shown at the bottom right.
    pending: Vec<Key>,
    count: Option<NonZeroUsize>,
    next_key: Option<NextKey>,
    prompt: Option<Prompt>,
    picker: Option<Picker>,
    /// Helix's `/` register: the last search.
    last_search: Option<String>,
    /// Helix's `"` register: the last yank.
    yanked: Vec<String>,
    status: Option<(String, bool)>,
    /// The screen and text area from the last draw, for commands that page or align the view.
    screen: Rect,
    view: Rect,
    quit: bool,
}

pub fn run(source: Source, config: &Config) -> Result<()> {
    let theme = match Theme::load(&config.theme, ColorSupport::detect()) {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!("{e}; using the default theme");
            Theme::load("default", ColorSupport::detect())?
        }
    };
    let loader = Loader::new(&theme)?;
    let mut app = App {
        files: (0..source.changes.len()).map(|_| None).collect(),
        context: HashMap::new(),
        source,
        cur: Cur::Change(0),
        mode: Mode::Normal,
        theme,
        loader,
        keymaps: Keymaps::default(),
        pending: vec![],
        count: None,
        next_key: None,
        prompt: None,
        picker: None,
        last_search: None,
        yanked: vec![],
        status: None,
        screen: Rect::default(),
        view: Rect::default(),
        quit: false,
    };
    // Raw mode and the alternate screen; the terminal is restored on a panic too.
    let mut terminal = ratatui::init();
    if config.editor.mouse {
        crossterm::execute!(std::io::stdout(), crossterm::event::EnableMouseCapture)?;
    }
    let result = app.event_loop(&mut terminal);
    if config.editor.mouse {
        let _ = crossterm::execute!(std::io::stdout(), crossterm::event::DisableMouseCapture);
    }
    ratatui::restore();
    result
}

impl App {
    fn event_loop(&mut self, terminal: &mut DefaultTerminal) -> Result<()> {
        let size = terminal.size()?;
        self.screen = Rect::new(0, 0, size.width, size.height);
        self.view = Rect::new(0, 0, size.width, size.height.saturating_sub(2));
        self.load(0);
        while !self.quit {
            self.load_preview();
            terminal.draw(|f| self.render(f))?;
            match event::read()? {
                Event::Key(k) if k.kind != KeyEventKind::Release => {
                    if self.key(Key::from(k)) == Some(AppAction::Suspend) {
                        suspend(terminal)?;
                    }
                }
                Event::Mouse(m) => self.mouse(m),
                _ => {} // resizes and anything else just redraw
            }
        }
        Ok(())
    }

    fn load(&mut self, i: usize) {
        if self.files[i].is_none() {
            self.files[i] = Some(File::change(&self.source.changes[i], &self.source, &self.loader));
        }
    }

    fn load_context(&mut self, path: &str) {
        if !self.context.contains_key(path) {
            let file = File::context(path, &self.source, &self.loader);
            self.context.insert(path.to_string(), file);
        }
    }

    /// The picker's preview needs its file loaded before drawing.
    fn load_preview(&mut self) {
        match self.picker.as_ref().and_then(|p| p.selected().cloned()) {
            Some(Target::Change(i)) => self.load(i),
            Some(Target::Path(p)) => self.load_context(&p),
            None => {}
        }
    }

    fn file(&self) -> &File {
        match &self.cur {
            Cur::Change(i) => self.files[*i].as_ref().expect("the current file is loaded"),
            Cur::Context(p) => &self.context[p],
        }
    }

    fn file_mut(&mut self) -> &mut File {
        match &self.cur {
            Cur::Change(i) => self.files[*i].as_mut().expect("the current file is loaded"),
            Cur::Context(p) => self.context.get_mut(p).expect("the current file is loaded"),
        }
    }

    fn open(&mut self, target: Target) {
        match target {
            Target::Change(i) => {
                self.load(i);
                self.cur = Cur::Change(i);
            }
            Target::Path(p) => {
                // A changed file opens as its change, with its diff.
                if let Some(i) = self.source.changes.iter().position(|c| c.path == p) {
                    return self.open(Target::Change(i));
                }
                self.load_context(&p);
                self.cur = Cur::Context(p);
            }
        }
        self.mode = Mode::Normal;
    }

    fn text_height(&self) -> usize {
        self.view.height.saturating_sub(self.file().notices.len() as u16) as usize
    }

    /// Handles one key. Returns the app action that needs the terminal (suspend), if any.
    fn key(&mut self, key: Key) -> Option<AppAction> {
        self.status = None;
        if let Some(picker) = &mut self.picker {
            let page = picker.page(Picker::areas(self.screen).0);
            match picker.key(key, page) {
                Outcome::Stay => {}
                Outcome::Close => self.picker = None,
                Outcome::Open(t) => {
                    self.picker = None;
                    self.open(t);
                }
            }
            return None;
        }
        if self.prompt.is_some() {
            self.prompt_key(key);
            return None;
        }
        if let Some(next) = self.next_key.take() {
            self.pending.clear();
            let ch = key.char().or(match key.code {
                KeyCode::Enter => Some('\n'),
                KeyCode::Tab => Some('\t'),
                _ => None,
            });
            if let Some(ch) = ch {
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

    fn prompt_key(&mut self, key: Key) {
        let Some(prompt) = &mut self.prompt else { return };
        match key.code {
            KeyCode::Esc => {
                if let Some(Prompt { kind: PromptKind::Search { origin, top, .. }, .. }) = self.prompt.take() {
                    let file = self.file_mut();
                    file.sel = origin;
                    file.top = top;
                }
            }
            KeyCode::Char('c') if key.mods.contains(KeyModifiers::CONTROL) => {
                self.prompt_key(Key { code: KeyCode::Esc, mods: KeyModifiers::NONE });
            }
            KeyCode::Enter => {
                let Some(Prompt { kind, line }) = self.prompt.take() else { return };
                match kind {
                    PromptKind::Command => self.typed_command(line.trim()),
                    PromptKind::Search { .. } => {
                        // An empty search repeats the last one, as in Helix.
                        if !line.is_empty() {
                            self.last_search = Some(line);
                        }
                        if commands::search_regex(self.last_search.as_deref().unwrap_or("")).is_none() {
                            self.status = Some(("Invalid regex".into(), true));
                        }
                    }
                }
            }
            KeyCode::Backspace => {
                if prompt.line.pop().is_none() {
                    return self.prompt_key(Key { code: KeyCode::Esc, mods: KeyModifiers::NONE });
                }
                self.search_preview();
            }
            KeyCode::Char(c) if !key.mods.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) => {
                prompt.line.push(c);
                self.search_preview();
            }
            _ => {}
        }
    }

    /// As you type a search, the selection jumps to the first match from where you started.
    fn search_preview(&mut self) {
        let Some(Prompt { kind: PromptKind::Search { forward, origin, top }, line }) = &self.prompt else { return };
        let (forward, origin, top) = (*forward, origin.clone(), *top);
        let Some(regex) = commands::search_regex(line) else { return };
        let file = self.file_mut();
        file.sel = origin;
        file.top = top;
        self.doc_command(|cx| commands::search(cx, &regex, forward, false));
    }

    fn app_command(&mut self, action: AppAction) -> Option<AppAction> {
        match action {
            AppAction::SelectMode => self.mode = Mode::Select,
            AppAction::NormalMode => self.mode = Mode::Normal,
            AppAction::CommandMode => self.prompt = Some(Prompt { kind: PromptKind::Command, line: String::new() }),
            AppAction::Search(forward) => {
                let file = self.file();
                let kind = PromptKind::Search { forward, origin: file.sel.clone(), top: file.top };
                self.prompt = Some(Prompt { kind, line: String::new() });
            }
            AppAction::SearchNext(forward) => match self.last_search.clone() {
                Some(q) => self.doc_command(|cx| commands::search_next(cx, &q, forward)),
                None => self.status = Some(("No previous search".into(), true)),
            },
            AppAction::SearchSelection(detect) => {
                let mut query = None;
                self.doc_command(|cx| query = Some(commands::search_selection(cx, detect)));
                if let Some(q) = query {
                    self.last_search = Some(q);
                }
            }
            AppAction::NextFile | AppAction::PrevFile => {
                let n = self.files.len();
                let step = self.count.map_or(1, NonZeroUsize::get) % n;
                let i = match self.cur {
                    Cur::Change(i) if action == AppAction::NextFile => (i + step) % n,
                    Cur::Change(i) => (i + n - step) % n,
                    // From a context file, the changes start again.
                    Cur::Context(_) if action == AppAction::NextFile => 0,
                    Cur::Context(_) => n - 1,
                };
                self.open(Target::Change(i));
            }
            AppAction::ChangedFilePicker => self.picker = Some(self.changed_file_picker()),
            AppAction::FilePicker => self.picker = Some(self.file_picker()),
            AppAction::ToggleCollapse => {
                let file = self.file_mut();
                let marker = "generated file, collapsed; enter expands";
                if file.notices.iter().any(|n| n == marker) || file.collapsed {
                    file.collapsed = !file.collapsed;
                }
            }
            AppAction::Yank | AppAction::YankToClipboard { .. } => {
                let file = self.file();
                let text = file.text.slice(..);
                let mut values: Vec<String> = file.sel.fragments(text).map(|f| f.into_owned()).collect();
                if let AppAction::YankToClipboard { main_only: true } = action {
                    values = vec![file.sel.primary().fragment(text).into_owned()];
                }
                let n = values.len();
                let (register, result) = match action {
                    AppAction::Yank => {
                        self.yanked = values;
                        ('"', Ok(()))
                    }
                    // Helix joins selections with newlines for the clipboard.
                    _ => ('+', clipboard::copy(&values.join("\n"))),
                };
                self.status = Some(match result {
                    Ok(()) => (format!("yanked {n} selection{} to register {register}", if n == 1 { "" } else { "s" }), false),
                    Err(e) => (format!("{e:#}"), true),
                });
                self.mode = Mode::Normal; // as Helix, yanking leaves select mode
            }
            AppAction::Suspend => return Some(AppAction::Suspend),
        }
        None
    }

    /// Click to move the cursor, drag to select, wheel to scroll; in the picker, the wheel moves
    /// and a click opens.
    fn mouse(&mut self, m: MouseEvent) {
        if let Some(picker) = &mut self.picker {
            let (list, _) = Picker::areas(self.screen);
            match m.kind {
                MouseEventKind::ScrollDown => {
                    picker.key(Key { code: KeyCode::Down, mods: KeyModifiers::NONE }, 1);
                }
                MouseEventKind::ScrollUp => {
                    picker.key(Key { code: KeyCode::Up, mods: KeyModifiers::NONE }, 1);
                }
                MouseEventKind::Down(MouseButton::Left) => {
                    if let Some(t) = picker.target_at(list, m.column, m.row) {
                        self.picker = None;
                        self.open(t);
                    }
                }
                _ => {}
            }
            return;
        }
        if self.prompt.is_some() {
            return;
        }
        match m.kind {
            MouseEventKind::ScrollDown | MouseEventKind::ScrollUp => {
                // Helix's editor.scroll-lines: 3, and the cursor stays in view.
                let down = m.kind == MouseEventKind::ScrollDown;
                self.doc_command(|cx| commands::scroll_lines(cx, 3, down));
            }
            MouseEventKind::Down(MouseButton::Left) | MouseEventKind::Drag(MouseButton::Left) => {
                let Some(pos) = self.pos_at(m.column, m.row) else { return };
                let drag = matches!(m.kind, MouseEventKind::Drag(_));
                self.doc_command(|cx| {
                    cx.sel = if drag {
                        let text = cx.text.slice(..);
                        let r = cx.sel.primary().put_cursor(text, pos, true);
                        cx.sel.clone().replace(cx.sel.primary_index(), r)
                    } else {
                        Selection::single(pos, pos)
                    };
                });
            }
            _ => {}
        }
    }

    /// The character under a screen cell in the file view.
    fn pos_at(&self, x: u16, y: u16) -> Option<usize> {
        let file = self.file();
        let header = file.notices.len() as u16;
        let gutter = file.gutter_width() as u16;
        if y < self.view.y + header || y >= self.view.bottom() || x < self.view.x + gutter || file.collapsed || file.binary {
            return None;
        }
        let text = file.text.slice(..);
        let line = file.top + (y - self.view.y - header) as usize;
        if line >= text.len_lines() {
            return Some(text.len_chars());
        }
        let col = file.left + (x - self.view.x - gutter) as usize;
        Some(view::pos_at_col(text, line, col, file.tab_width))
    }

    /// Helix's changed-file picker: a change column, then the path.
    fn changed_file_picker(&self) -> Picker {
        let t = &self.theme;
        let items = self
            .source
            .changes
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let (tag, style) = match c.kind() {
                    Kind::Added => ("+ added", t.get("diff.plus")),
                    Kind::Deleted => ("- deleted", t.get("diff.minus")),
                    Kind::Renamed => ("> renamed", t.get("diff.delta.moved")),
                    Kind::Modified | Kind::ModeChanged => ("~ modified", t.get("diff.delta")),
                };
                let text = match (c.kind(), &c.old_path) {
                    (Kind::Renamed, Some(o)) => format!("{o} -> {}", c.path),
                    _ => c.path.clone(),
                };
                Item { tags: vec![(tag.into(), style)], text, target: Target::Change(i) }
            })
            .collect();
        Picker::new(vec!["change", "path"], items)
    }

    /// Helix's file picker over the repo (or folder), honouring `.gitignore`, without hidden
    /// files. Files open read-only, for context.
    fn file_picker(&self) -> Picker {
        let root = &self.source.root;
        let mut items = vec![];
        // ponytail: walks synchronously; Helix streams results in. Capped as Helix caps it.
        let walker = ignore::WalkBuilder::new(root).hidden(true).require_git(false).build();
        for entry in walker.flatten().take(100_000) {
            if entry.file_type().is_some_and(|t| t.is_file() || t.is_symlink()) {
                if let Ok(rel) = entry.path().strip_prefix(root) {
                    let p = rel.to_string_lossy().into_owned();
                    // A changed file previews and opens as its change.
                    let target = match self.source.changes.iter().position(|c| c.path == p) {
                        Some(i) => Target::Change(i),
                        None => Target::Path(p.clone()),
                    };
                    items.push(Item { tags: vec![], text: p, target });
                }
            }
        }
        items.sort_by(|a, b| a.text.cmp(&b.text));
        Picker::new(vec!["path"], items)
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
        let (count, extend, height, width) =
            (self.count, self.mode == Mode::Select, self.text_height(), self.view.width);
        let file = self.file_mut();
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
            center: false,
        };
        f(&mut cx);
        let (sel, top, status, center) = (cx.sel.ensure_invariants(file.text.slice(..)), cx.top, cx.status, cx.center);
        file.sel = sel;
        file.top = top;
        if status.is_some() {
            self.status = status;
        }
        self.scroll_to_cursor(center);
    }

    /// Keeps the primary cursor `scrolloff` lines and columns inside the view; with `center`,
    /// a cursor outside the view is centred instead, as Helix does after a search.
    fn scroll_to_cursor(&mut self, center: bool) {
        let height = self.text_height();
        let width = (self.view.width as usize).saturating_sub(self.file().gutter_width());
        let file = self.file_mut();
        let text = file.text.slice(..);
        let cursor = file.sel.primary().cursor(text);
        let line = text.char_to_line(cursor);
        let so = commands::SCROLLOFF.min(height.saturating_sub(1) / 2);
        let outside = line < file.top + so || (height > 0 && line + so >= file.top + height);
        if center && outside {
            file.top = line.saturating_sub(height / 2);
        } else if line < file.top + so {
            file.top = line.saturating_sub(so);
        } else if height > 0 && line + so >= file.top + height {
            file.top = line + so + 1 - height;
        }
        let col = view::visual_col(text, cursor, file.tab_width);
        let so = commands::SCROLLOFF.min(width.saturating_sub(1) / 2);
        if col < file.left + so {
            file.left = col.saturating_sub(so);
        } else if width > 0 && col + so >= file.left + width {
            file.left = col + so + 1 - width;
        }
    }

    fn render(&mut self, f: &mut Frame) {
        let area = f.area();
        self.screen = area;
        let buf = f.buffer_mut();
        buf.set_style(area, self.theme.get("ui.background"));
        if area.height < 3 {
            return;
        }
        self.view = Rect::new(0, 0, area.width, area.height - 2);
        let file = self.file();
        let styles = self.selection_styles();
        view::draw(buf, self.view, file, file.top, file.left, &self.loader, &self.theme, Some(styles));
        self.render_statusline(buf, Rect::new(0, area.height - 2, area.width, 1));
        self.render_command_line(buf, Rect::new(0, area.height - 1, area.width, 1));
        self.render_picker(buf, area);
        self.render_help(f, area);
    }

    fn render_picker(&mut self, buf: &mut Buffer, area: Rect) {
        let Some(picker) = &mut self.picker else { return };
        let (list, preview) = Picker::areas(area);
        picker.render(buf, list, &self.theme);
        let Some(preview) = preview else { return };
        let inner = picker::preview_frame(buf, preview, &self.theme);
        let file = match picker.selected() {
            Some(Target::Change(i)) => self.files[*i].as_ref(),
            Some(Target::Path(p)) => self.context.get(p),
            None => None,
        };
        if let Some(file) = file {
            // Centre the first change, as Helix centres a preview on its range.
            let top = file.hunks.first().map_or(0, |h| (h.after.start as usize).saturating_sub(inner.height as usize / 3));
            view::draw(buf, inner, file, top, 0, &self.loader, &self.theme, None);
        }
    }

    /// (selection, primary selection, cursor, primary cursor), as Helix picks them per mode.
    fn selection_styles(&self) -> view::SelectionStyles {
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
        let base = self.theme.get("ui.statusline");
        buf.set_style(area, base);
        let file = self.file();
        let (path, which) = match &self.cur {
            Cur::Change(i) => (self.source.changes[*i].path.as_str(), format!("{}/{} files", i + 1, self.files.len())),
            Cur::Context(p) => (p.as_str(), "context".to_string()),
        };
        let mode = if self.mode == Mode::Select { " SEL " } else { " NOR " };
        let left = format!("{mode} {}", clean(path));
        buf.set_stringn(area.x, area.y, &left, area.width as usize, base);

        let text = file.text.slice(..);
        let cursor = file.sel.primary().cursor(text);
        let line = text.char_to_line(cursor);
        let col = cursor - text.line_to_char(line);
        let sels = file.sel.len();
        let right =
            format!(" {which}  {sels} sel{}  {}:{} ", if sels == 1 { "" } else { "s" }, line + 1, col + 1);
        let w = right.width() as u16;
        if w + left.width() as u16 + 1 < area.width {
            buf.set_string(area.x + area.width - w, area.y, &right, base);
        }
    }

    fn render_command_line(&self, buf: &mut Buffer, area: Rect) {
        let t = &self.theme;
        if let Some(p) = &self.prompt {
            let prefix = match p.kind {
                PromptKind::Command => ":",
                PromptKind::Search { .. } => "search:",
            };
            let shown = format!("{prefix}{}", clean(&p.line));
            buf.set_stringn(area.x, area.y, &shown, area.width as usize, t.get("ui.text"));
            let x = area.x + shown.width() as u16;
            if x < area.right() {
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
        let width = width.min(area.width);
        let height = (lines.len() as u16 + 2).min(area.height.saturating_sub(2));
        let rect = Rect::new(area.width - width, area.height.saturating_sub(2 + height), width, height);
        let t = &self.theme;
        let popup = t.get("ui.popup.info");
        f.render_widget(Clear, rect);
        f.render_widget(Block::default().borders(Borders::ALL).title(title).style(popup).border_style(popup), rect);
        let text_style: Style = popup.patch(t.get("ui.text.info"));
        for (i, l) in lines.iter().enumerate().take(height.saturating_sub(2) as usize) {
            f.buffer_mut().set_stringn(rect.x + 2, rect.y + 1 + i as u16, l, width.saturating_sub(3) as usize, text_style);
        }
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
