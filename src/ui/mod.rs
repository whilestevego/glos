//! The reader: the full-file view, the statusline, the command line, the key help panel and the
//! pickers.

mod commands;
mod keymap;
mod picker;
mod view;

use std::{collections::HashMap, num::NonZeroUsize};

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use helix_core::{Selection, char_idx_at_visual_offset, text_annotations::TextAnnotations, visual_offset_from_anchor};
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
    /// `s`, `S`, `K`, `A-K`: previews as you type, as Helix's regex prompts do.
    Regex { action: commands::RegexAction, origin: Selection },
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
    /// Soft wrap: `[editor.soft-wrap] enable`, or `:toggle soft-wrap`.
    wrap: bool,
    /// Helix's jumplist: where `gg`, `G`, opening a file… came from. `jump` is the position in it.
    jumps: Vec<(Cur, Selection)>,
    jump: usize,
    /// The last `f`/`t`, text object or `]g`-style motion, for `A-.`.
    last_motion: Option<std::rc::Rc<dyn Fn(&mut Ctx)>>,
    /// Keys being recorded into the `@` register (`Q`), and the last recording.
    recording: Option<Vec<Key>>,
    macro_keys: Vec<Key>,
    replaying: bool,
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
    let mut app = App::new(source, config, theme)?;
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
    fn new(source: Source, config: &Config, theme: Theme) -> Result<App> {
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
            wrap: config.editor.soft_wrap.enable,
            jumps: vec![],
            jump: 0,
            last_motion: None,
            recording: None,
            macro_keys: vec![],
            replaying: false,
            status: None,
            screen: Rect::default(),
            view: Rect::default(),
            quit: false,
        };
        app.load(0);
        Ok(app)
    }

    /// One frame: the picker's preview is loaded first.
    fn draw(&mut self, f: &mut Frame) {
        self.load_preview();
        self.render(f);
    }
    fn event_loop(&mut self, terminal: &mut DefaultTerminal) -> Result<()> {
        let size = terminal.size()?;
        self.screen = Rect::new(0, 0, size.width, size.height);
        self.view = Rect::new(0, 0, size.width, size.height.saturating_sub(2));
        let mut first = true;
        while !self.quit {
            terminal.draw(|f| self.draw(f))?;
            if std::mem::take(&mut first) {
                tracing::info!("first screen after {:?}", crate::STARTED.elapsed());
            }
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
        let _ = self.push_jump();
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
        if let Some(rec) = &mut self.recording {
            rec.push(key);
        }
        if !self.replaying {
            self.status = None;
        }
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
                let motion: std::rc::Rc<dyn Fn(&mut Ctx)> = match next {
                    NextKey::Char(f, _) => std::rc::Rc::new(move |cx| f(cx, ch)),
                    NextKey::Textobject(around) => std::rc::Rc::new(move |cx| commands::select_textobject(cx, around, ch)),
                };
                self.doc_command(|cx| motion(cx));
                self.last_motion = Some(motion);
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
        let mut sticky: Option<usize> = None;
        for (depth, k) in self.pending.iter().chain(std::iter::once(&key)).enumerate() {
            match menu.get(*k) {
                Some(Node::Menu(m)) => {
                    menu = m;
                    node = None;
                    if m.sticky {
                        sticky = Some(depth + 1);
                    }
                }
                Some(Node::Cmd(c)) => node = Some(*c),
                None => {
                    // Unbound: drop the sequence (and leave a sticky menu), as Helix does.
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
        // In a sticky menu the keys up to it stay typed, so the next key is in it again.
        let keep: Vec<Key> = sticky.map(|n| self.pending[..n.min(self.pending.len())].to_vec()).unwrap_or_default();
        self.pending = keep;
        let result = match command.action {
            Action::Doc(f) => {
                // As Helix: these jumps go on the jumplist, and these motions repeat with A-.
                if matches!(command.name, "goto_file_start" | "goto_last_line" | "goto_line") {
                    let _ = self.push_jump();
                }
                if matches!(
                    command.name,
                    "goto_next_change" | "goto_prev_change" | "goto_next_paragraph" | "goto_prev_paragraph"
                ) {
                    self.last_motion = Some(std::rc::Rc::new(f));
                }
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
            KeyCode::Esc => match self.prompt.take() {
                Some(Prompt { kind: PromptKind::Search { origin, top, .. }, .. }) => {
                    let file = self.file_mut();
                    file.sel = origin;
                    file.top = top;
                }
                Some(Prompt { kind: PromptKind::Regex { origin, .. }, .. }) => self.file_mut().sel = origin,
                _ => {}
            },
            KeyCode::Char('c') if key.mods.contains(KeyModifiers::CONTROL) => {
                self.prompt_key(Key { code: KeyCode::Esc, mods: KeyModifiers::NONE });
            }
            KeyCode::Enter => {
                let Some(Prompt { kind, line }) = self.prompt.take() else { return };
                match kind {
                    PromptKind::Command => self.typed_command(line.trim()),
                    PromptKind::Regex { action, origin } => {
                        // Applied once more from where it started, so errors show.
                        if let Some(regex) = commands::search_regex(&line) {
                            self.file_mut().sel = origin;
                            self.doc_command(|cx| commands::regex_selections(cx, action, &regex));
                        }
                    }
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

    /// As you type a search, the selection jumps to the first match from where you started; a
    /// regex prompt applies to the selections it started from.
    fn search_preview(&mut self) {
        if let Some(Prompt { kind: PromptKind::Regex { action, origin }, line }) = &self.prompt {
            let (action, origin) = (*action, origin.clone());
            let Some(regex) = commands::search_regex(line) else { return };
            self.file_mut().sel = origin;
            self.doc_command(|cx| commands::regex_selections(cx, action, &regex));
            self.status = None;
            return;
        }
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
            AppAction::RegexPrompt(action) => {
                let kind = PromptKind::Regex { action, origin: self.file().sel.clone() };
                self.prompt = Some(Prompt { kind, line: String::new() });
            }
            AppAction::SaveSelection => {
                let _ = self.push_jump();
                self.status = Some(("Selection saved to jumplist".into(), false));
            }
            AppAction::JumpBackward | AppAction::JumpForward => {
                let count = self.count.map_or(1, NonZeroUsize::get);
                let target = if action == AppAction::JumpForward {
                    self.jump_forward(count)
                } else {
                    self.jump_backward(count)
                };
                let Some((cur, sel)) = target else { return None };
                self.cur = cur;
                if let Cur::Change(i) = self.cur {
                    self.load(i);
                }
                let file = self.file_mut();
                file.sel = sel.ensure_invariants(file.text.slice(..));
                self.scroll_to_cursor(true);
            }
            AppAction::RepeatLastMotion => {
                if let Some(m) = self.last_motion.clone() {
                    for _ in 0..self.count.map_or(1, NonZeroUsize::get) {
                        self.doc_command(|cx| m(cx));
                    }
                }
            }
            AppAction::RecordMacro => match self.recording.take() {
                Some(mut keys) => {
                    keys.pop(); // the Q that stopped it
                    self.macro_keys = keys;
                    self.status = Some(("Recorded to register [@]".into(), false));
                }
                None => {
                    self.recording = Some(vec![]);
                    self.status = Some(("Recording to register [@]".into(), false));
                }
            },
            AppAction::ReplayMacro => {
                if self.replaying {
                    self.status = Some(("Cannot replay from register [@] because already replaying".into(), true));
                    return None;
                }
                let keys = self.macro_keys.clone();
                self.replaying = true;
                for _ in 0..self.count.map_or(1, NonZeroUsize::get) {
                    self.count = None;
                    for k in &keys {
                        self.key(*k);
                    }
                }
                self.replaying = false;
            }
            AppAction::Suspend => return Some(AppAction::Suspend),
        }
        None
    }

    /// Helix's `push_jump`: the current place goes on the jumplist, dropping anything ahead
    /// (helix-view `JumpList::push_impl`, capacity 30). Returns how many fell off the front.
    fn push_jump(&mut self) -> usize {
        let entry = (self.cur.clone(), self.file().sel.clone());
        self.jumps.truncate(self.jump);
        let mut removed = 0;
        if self.jumps.last() != Some(&entry) {
            while self.jumps.len() >= 30 {
                self.jumps.remove(0);
                removed += 1;
            }
            self.jumps.push(entry);
            self.jump = self.jumps.len();
        }
        removed
    }

    fn jump_forward(&mut self, count: usize) -> Option<(Cur, Selection)> {
        if self.jump + count < self.jumps.len() {
            self.jump += count;
            self.jumps.get(self.jump).cloned()
        } else {
            None
        }
    }

    /// helix-view `JumpList::backward`: from the newest jump, where you are is saved first, so
    /// `C-i` comes back; a jump to where you already are is skipped.
    fn jump_backward(&mut self, count: usize) -> Option<(Cur, Selection)> {
        let mut current = self.jump.checked_sub(count)?;
        if self.jump == self.jumps.len() {
            current = current.saturating_sub(self.push_jump());
        }
        self.jump = current;
        let here = (self.cur.clone(), self.file().sel.clone());
        if self.jumps.get(self.jump)? == &here {
            self.jump = self.jump.checked_sub(1)?;
        }
        self.jumps.get(self.jump).cloned()
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

    /// The character under a screen cell in the file view (Helix's `char_idx_at_visual_offset`).
    fn pos_at(&self, x: u16, y: u16) -> Option<usize> {
        let file = self.file();
        let header = file.notices.len() as u16;
        let gutter = file.gutter_width() as u16;
        if y < self.view.y + header || y >= self.view.bottom() || x < self.view.x + gutter || file.collapsed || file.binary {
            return None;
        }
        let text = file.text.slice(..);
        let fmt = view::text_format(file, self.view.width.saturating_sub(gutter), self.wrap);
        let left = if fmt.soft_wrap { 0 } else { file.left };
        let row = (y - self.view.y - header) as isize;
        let col = left + (x - self.view.x - gutter) as usize;
        let anchor = text.line_to_char(file.top.min(text.len_lines() - 1));
        let (pos, _) = char_idx_at_visual_offset(text, anchor, row, col, &fmt, &TextAnnotations::default());
        Some(pos)
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
            "toggle" | "toggle-option" => match _args.trim() {
                "soft-wrap" | "soft-wrap.enable" => {
                    self.wrap = !self.wrap;
                    self.status = Some((format!("'soft-wrap.enable' is now set to {}", self.wrap), false));
                    self.scroll_to_cursor(false);
                }
                other => self.status = Some((format!("unknown option '{other}'"), true)),
            },
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
        let (count, extend, height, wrap) = (self.count, self.mode == Mode::Select, self.text_height(), self.wrap);
        let width = self.view.width.saturating_sub(self.file().gutter_width() as u16);
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
            left: file.left,
            height,
            fmt: view::text_format(file, width, wrap),
            status: None,
            center: false,
            keep_top: false,
        };
        f(&mut cx);
        let (sel, top, left, status, center, keep_top) =
            (cx.sel.ensure_invariants(file.text.slice(..)), cx.top, cx.left, cx.status, cx.center, cx.keep_top);
        file.sel = sel;
        file.top = top;
        file.left = left;
        if status.is_some() {
            self.status = status;
        }
        if !keep_top {
            self.scroll_to_cursor(center);
        }
    }

    /// Keeps the primary cursor `scrolloff` rows and columns inside the view; with `center`,
    /// a cursor outside the view is centred instead, as Helix does after a search. Rows are
    /// visual rows, so a wrapped line counts as more than one.
    fn scroll_to_cursor(&mut self, center: bool) {
        let height = self.text_height();
        let width = self.view.width.saturating_sub(self.file().gutter_width() as u16);
        let wrap = self.wrap;
        let file = self.file_mut();
        let fmt = view::text_format(file, width, wrap);
        let annotations = TextAnnotations::default();
        let text = file.text.slice(..);
        let cursor = file.sel.primary().cursor(text);
        let line = text.char_to_line(cursor);
        let so = commands::SCROLLOFF.min(height.saturating_sub(1) / 2);
        // The cursor's row below the top of the view, if it's within `height` rows.
        let row_of = |top: usize| {
            if line < top {
                return None;
            }
            visual_offset_from_anchor(text, text.line_to_char(top), cursor, &fmt, &annotations, height)
                .ok()
                .map(|(p, _)| p.row)
        };
        let outside = line < file.top + so || row_of(file.top).is_none_or(|r| r + so >= height);
        if center && outside {
            file.top = line;
            // Back up until the cursor is about half way down.
            while file.top > 0 && row_of(file.top - 1).is_some_and(|r| r <= height / 2) {
                file.top -= 1;
            }
        } else if line < file.top + so {
            file.top = line.saturating_sub(so);
        } else {
            while file.top < line && row_of(file.top).is_none_or(|r| r + so >= height) {
                file.top += 1;
            }
        }
        if fmt.soft_wrap {
            file.left = 0;
            return;
        }
        let col = view::visual_col(text, cursor, file.tab_width);
        let width = width as usize;
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
        view::draw(buf, self.view, file, file.top, file.left, self.wrap, &self.loader, &self.theme, Some(styles));
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
            view::draw(buf, inner, file, top, 0, self.wrap, &self.loader, &self.theme, None);
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
                PromptKind::Regex { action, .. } => action.prompt(),
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

#[cfg(test)]
mod tests {
    //! Screen snapshots of the parts that must look like Helix: the view with its gutter and
    //! statusline, the key help panel, the picker and the prompt.
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};

    fn app(dir: &std::path::Path) -> App {
        let (old, new) = (dir.join("old.rs"), dir.join("main.rs"));
        std::fs::write(&old, "fn main() {\n    println!(\"hi\");\n}\n\nfn gone() {}\n").unwrap();
        std::fs::write(&new, "fn main() {\n    println!(\"hello\");\n    let x = 1;\n}\n").unwrap();
        let source = Source::diff(&old, &new).unwrap();
        let theme = Theme::load("default", ColorSupport::TrueColor).unwrap();
        App::new(source, &Config::default(), theme).unwrap()
    }

    /// Types Helix-notation keys (`g`, `space`, `C-d`…), then draws and returns the screen.
    fn screen(app: &mut App, keys: &str) -> String {
        let mut term = Terminal::new(TestBackend::new(72, 14)).unwrap();
        term.draw(|f| app.draw(f)).unwrap();
        for k in keys.split_whitespace() {
            app.key(Key::parse(k).unwrap_or_else(|| panic!("key {k}")));
        }
        term.draw(|f| app.draw(f)).unwrap();
        let buf = term.backend().buffer();
        (0..buf.area.height)
            .map(|y| (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect::<String>().trim_end().to_string())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn file_view() {
        let d = tempfile::tempdir().unwrap();
        insta::assert_snapshot!(screen(&mut app(d.path()), ""));
    }

    #[test]
    fn key_help_panel() {
        let d = tempfile::tempdir().unwrap();
        insta::assert_snapshot!(screen(&mut app(d.path()), "g"));
    }

    #[test]
    fn select_mode_and_count() {
        let d = tempfile::tempdir().unwrap();
        insta::assert_snapshot!(screen(&mut app(d.path()), "v 2 w"));
    }

    #[test]
    fn changed_file_picker() {
        let d = tempfile::tempdir().unwrap();
        let mut term_app = app(d.path());
        insta::assert_snapshot!(screen(&mut term_app, "space g"));
    }

    #[test]
    fn search_prompt() {
        let d = tempfile::tempdir().unwrap();
        insta::assert_snapshot!(screen(&mut app(d.path()), "/ l e t"));
    }
}
