//! The reader: the full-file view, the statusline, the command line, the key help panel and the
//! pickers.

mod comments;
mod dashboard;
mod diffview;
mod keymap;
mod picker;
mod tree;
mod view;

use std::{
    collections::{HashMap, HashSet},
    num::NonZeroUsize,
    time::Duration,
};

use anyhow::{Context as _, Result};
use crossterm::event::{
    self, Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use glos_helix::{
    commands::{self, Ctx},
    layout::{TextAnnotations, char_idx_at_visual_offset, visual_offset_from_anchor},
    syntax::Loader,
    text::Selection,
};
use ratatui::{
    DefaultTerminal, Frame,
    buffer::Buffer,
    layout::Rect,
    style::Style,
    widgets::{Block, Borders, Clear, Widget},
};
use rusqlite::Connection;
use unicode_width::UnicodeWidthStr;

use crate::{
    clipboard,
    config::Config,
    review::{self, Anchor, Review, Thread, Verdict, YOU},
    source::{Content, Kind, Source},
    theme::{ColorSupport, Theme},
};
use comments::{CommentBox, Place, ThreadLines, Threads};
use keymap::{Action, AppAction, Key, Keymaps, Node};
use picker::{Item, Outcome, Picker, Target};
use view::{File, clean};

/// A motion that `A-.` can repeat.
type Motion = std::rc::Rc<dyn Fn(&mut Ctx)>;

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
    Search {
        forward: bool,
        origin: Selection,
        top: usize,
    },
    /// `s`, `S`, `K`, `A-K`: previews as you type, as Helix's regex prompts do.
    Regex {
        action: commands::RegexAction,
        origin: Selection,
    },
}

struct Prompt {
    kind: PromptKind,
    line: String,
    /// While tabbing through `:` completions: what was typed, and which completion is shown.
    completion: Option<(String, usize)>,
}

impl Prompt {
    fn new(kind: PromptKind) -> Prompt {
        Prompt {
            kind,
            line: String::new(),
            completion: None,
        }
    }
}

/// How the file on screen is shown: whole with the gutter, as hunks, or old beside new.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ViewKind {
    File,
    Unified,
    Split,
}

impl ViewKind {
    fn name(self) -> &'static str {
        match self {
            ViewKind::File => "file",
            ViewKind::Unified => "unified",
            ViewKind::Split => "split",
        }
    }
}

/// The unified or side-by-side document for one file, built when it's shown.
struct Alt {
    cur: Cur,
    kind: ViewKind,
    doc: File,
    /// The old side, for side by side.
    left: Option<File>,
}

/// Which changes are reviewed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Scope {
    /// Branch vs base for git; old vs new for two files or folders.
    Base,
    /// Against the snapshot of a round: what changed since you submitted it.
    SinceReview(u32),
    /// One commit of the branch, by its index (oldest first).
    Commit(usize),
}

/// `:filter`: which of the changed files to step through.
#[derive(Clone, Debug)]
pub enum Filter {
    Glob(String, globset::GlobMatcher),
    /// Only files with open threads.
    Threads,
}

/// The comment box's height, borders included.
const BOX_HEIGHT: u16 = 8;

/// `:` commands, with their aliases and descriptions, for running and completing them.
const TYPABLE: &[(&str, &[&str], &str)] = &[
    (
        "quit",
        &["q", "qa"],
        "Close glos; reminds you once of unsent comments",
    ),
    (
        "quit!",
        &["q!", "qa!"],
        "Close glos now; unsent comments are kept",
    ),
    ("comment", &[], "Comment on the selected lines"),
    ("review-comment", &[], "Comment on the whole review"),
    ("reply", &[], "Reply to the thread under the cursor"),
    (
        "edit-comment",
        &[],
        "Edit your latest comment on the thread under the cursor",
    ),
    (
        "submit",
        &[],
        "Submit the review to the agent: approve, changes or comment",
    ),
    ("editor", &[], "Edit the open comment in $VISUAL or $EDITOR"),
    (
        "view",
        &[],
        "Show the file whole, unified, or split side by side",
    ),
    (
        "scope",
        &[],
        "Review the branch (base), since your last review, or commit by commit",
    ),
    (
        "filter",
        &[],
        "Only files matching a glob, or with open threads; empty for all",
    ),
    ("toggle-option", &["toggle"], "Toggle an option: soft-wrap"),
    ("dashboard", &[], "Every open review on this machine"),
    (
        "close",
        &[],
        "Close the review, once everything is sent, and quit",
    ),
];

/// What `:` can complete `line` with: where the completed word starts, and the words with
/// their descriptions. Command names complete first, then `submit`'s and `toggle`'s argument.
fn completions(line: &str) -> (usize, Vec<(String, &'static str)>) {
    match line.split_once(' ') {
        None => (
            0,
            TYPABLE
                .iter()
                .filter(|(name, aliases, _)| {
                    name.starts_with(line) || aliases.iter().any(|a| a.starts_with(line))
                })
                .map(|(name, _, doc)| (name.to_string(), *doc))
                .collect(),
        ),
        Some((cmd, arg)) => {
            let args: &[(&str, &'static str)] = match cmd {
                "submit" => &[
                    ("approve", "Approve"),
                    ("changes", "Request changes"),
                    ("comment", "Send comments without a verdict"),
                ],
                "toggle" | "toggle-option" => &[("soft-wrap", "Soft-wrap long lines")],
                "view" => &[
                    ("file", "The whole file, changes in the gutter"),
                    ("unified", "Changes with three lines around them"),
                    ("split", "Old and new side by side"),
                ],
                "scope" => &[
                    ("base", "The branch against its base (or old against new)"),
                    ("since", "What changed since your last submit"),
                    ("commits", "One commit at a time"),
                ],
                "filter" => &[("threads", "Only files with open threads")],
                _ => &[],
            };
            (
                cmd.len() + 1,
                args.iter()
                    .filter(|(a, _)| a.starts_with(arg.trim_start()))
                    .map(|(a, d)| (a.to_string(), *d))
                    .collect(),
            )
        }
    }
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
    last_motion: Option<Motion>,
    /// Keys being recorded into the `@` register (`Q`), and the last recording.
    recording: Option<Vec<Key>>,
    macro_keys: Vec<Key>,
    replaying: bool,
    status: Option<(String, bool)>,
    /// The screen and text area from the last draw, for commands that page or align the view.
    screen: Rect,
    view: Rect,
    quit: bool,
    /// The store, and the review of this source in it.
    conn: Connection,
    review: Review,
    threads: Vec<Thread>,
    unsent: usize,
    /// Threads opened from their one-line marker.
    expanded: HashSet<String>,
    comment_box: Option<CommentBox>,
    /// Where the comment box was drawn, and each thread row (screen row, thread index).
    box_area: Rect,
    thread_hits: Vec<(u16, usize)>,
    /// Quitting with unsent comments reminds you once.
    reminded: bool,
    /// `:editor` asked for; the event loop runs it, since it needs the terminal.
    run_editor: bool,
    /// The store's `data_version` when last read: it changes when another process writes.
    data_version: i64,
    /// Threads placed onto versions they weren't made on, by (thread, generation, new side).
    placed: std::cell::RefCell<HashMap<PlacedKey, Option<Anchor>>>,
    /// Goes up whenever threads or files are read again, so `placed` is worked out afresh.
    generation: u64,
    /// Watches the files for changes (none in tests, which call `files_changed`).
    watch: Option<Watch>,
    /// When files last changed, until they settle.
    changed_at: Option<std::time::Instant>,
    view_kind: ViewKind,
    alt: Option<Alt>,
    /// The old side's area, in the side-by-side view.
    left_view: Rect,
    scope: Scope,
    /// The branch's commits, for the per-commit scope.
    commits: Vec<crate::source::Commit>,
    filter: Option<Filter>,
    /// The last round submitted, for "since last review".
    last_round: Option<u32>,
    tree: Option<tree::Tree>,
    tree_area: Rect,
    /// While the palette is open: the picker it covers, to go back to.
    palette: Option<Option<Picker>>,
    /// A review chosen in the dashboard: glos closes this one and opens it.
    pub switch: Option<Source>,
}

/// A palette row: the keys, what they do, and the command to run (none for a key only listed).
type KeyRow = (String, &'static str, Option<&'static str>);

/// What waits until the comment box closes: changing file, pickers and the review's actions.
fn blocked_in_box(action: AppAction) -> bool {
    use AppAction as A;
    matches!(
        action,
        A::NextFile
            | A::PrevFile
            | A::FilePicker
            | A::ChangedFilePicker
            | A::ToggleCollapse
            | A::JumpForward
            | A::JumpBackward
            | A::SaveSelection
            | A::Comment
            | A::ReviewComment
            | A::Reply
            | A::EditComment
            | A::Submit(_)
            | A::ThreadPicker { .. }
            | A::NextThread(_)
            | A::EdgeThread(_)
            | A::NextUnread(_)
            | A::ToggleResolved
            | A::GotoSuggestion
            | A::View(_)
            | A::Scope(_)
            | A::NextCommit(_)
            | A::FilterPrompt
            | A::FileTree
            | A::Dashboard
    )
}

/// A thread placed onto a version: (thread id, generation, the new side's commit).
type PlacedKey = (String, u64, Option<String>);

/// After this long without file changes, threads whose code is gone are marked outdated.
const SETTLE: Duration = Duration::from_secs(3);

/// File events from `notify`, debounced; anything under `.git` or ignored by the root
/// `.gitignore` doesn't count.
struct Watch {
    _debouncer: notify_debouncer_full::Debouncer<
        notify::RecommendedWatcher,
        notify_debouncer_full::NoCache,
    >,
    rx: std::sync::mpsc::Receiver<()>,
}

fn watch(root: &std::path::Path) -> Result<Watch> {
    // ponytail: only the root .gitignore; nested ones would need the ignore crate's walker.
    let (ignored, _) = ignore::gitignore::Gitignore::new(root.join(".gitignore"));
    let root_owned = root.to_path_buf();
    let (tx, rx) = std::sync::mpsc::channel();
    // No file-id cache: it walks every path under the root at start (target/ included, ~1 s
    // on this repo) only to pair up renames, and glos reads the review again on any change.
    let mut debouncer = notify_debouncer_full::new_debouncer_opt(
        Duration::from_millis(250),
        None,
        move |res: notify_debouncer_full::DebounceEventResult| {
            let Ok(events) = res else { return };
            let counts = |p: &std::path::Path| {
                let rel = p.strip_prefix(&root_owned).unwrap_or(p);
                !rel.components().any(|c| c.as_os_str() == ".git")
                    && !ignored
                        .matched_path_or_any_parents(rel, p.is_dir())
                        .is_ignore()
            };
            if events.iter().any(|e| e.paths.iter().any(|p| counts(p))) {
                let _ = tx.send(());
            }
        },
        notify_debouncer_full::NoCache,
        notify::Config::default(),
    )?;
    debouncer.watch(root, notify::RecursiveMode::Recursive)?;
    Ok(Watch {
        _debouncer: debouncer,
        rx,
    })
}

/// What glos opens first.
pub enum Start {
    Review(Box<Source>),
    Dashboard,
}

pub fn run(start: Start, config: &Config, conn: Connection) -> Result<()> {
    let theme = || match Theme::load(&config.theme, ColorSupport::detect()) {
        Ok(t) => Ok(t),
        Err(e) => {
            tracing::warn!("{e}; using the default theme");
            Theme::load("default", ColorSupport::detect())
        }
    };
    // Raw mode and the alternate screen; the terminal is restored on a panic too.
    let mut terminal = ratatui::init();
    crossterm::execute!(std::io::stdout(), crossterm::event::EnableFocusChange)?;
    if config.editor.mouse {
        crossterm::execute!(std::io::stdout(), crossterm::event::EnableMouseCapture)?;
    }
    let result = (|| -> Result<()> {
        let mut conn = conn;
        let mut next = Some(start);
        // One review at a time; the dashboard switches to another.
        while let Some(start) = next.take() {
            let source = match start {
                Start::Review(s) => *s,
                Start::Dashboard => match dashboard::run(&mut terminal, &conn, &theme()?)? {
                    Some(s) => s,
                    None => break,
                },
            };
            let mut app = App::new(source, config, theme()?, conn)?;
            match watch(&app.source.root) {
                Ok(w) => app.watch = Some(w),
                Err(e) => tracing::warn!("not watching files: {e:#}"),
            }
            if let Err(e) = review::set_viewer(&app.conn, Some(&app.review.id)) {
                tracing::warn!("not registered as a viewer: {e:#}");
            }
            let result = app.event_loop(&mut terminal, config.editor.mouse);
            let _ = review::set_viewer(&app.conn, None);
            result?;
            next = app.switch.take().map(|s| Start::Review(Box::new(s)));
            conn = app.conn;
        }
        Ok(())
    })();
    if config.editor.mouse {
        let _ = crossterm::execute!(std::io::stdout(), crossterm::event::DisableMouseCapture);
    }
    let _ = crossterm::execute!(std::io::stdout(), crossterm::event::DisableFocusChange);
    ratatui::restore();
    result
}

impl App {
    fn new(source: Source, config: &Config, theme: Theme, conn: Connection) -> Result<App> {
        let loader = Loader::new(theme.scopes())?;
        let review = Review::open(&conn, &source)?;
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
            conn,
            review,
            threads: vec![],
            unsent: 0,
            expanded: HashSet::new(),
            comment_box: None,
            box_area: Rect::default(),
            thread_hits: vec![],
            reminded: false,
            run_editor: false,
            data_version: 0,
            placed: Default::default(),
            generation: 0,
            watch: None,
            changed_at: None,
            view_kind: ViewKind::File,
            alt: None,
            left_view: Rect::default(),
            scope: Scope::Base,
            commits: vec![],
            filter: None,
            last_round: None,
            tree: None,
            tree_area: Rect::default(),
            palette: None,
            switch: None,
        };
        app.load(0);
        app.reload();
        app.reanchor(true);
        // The file tree starts open, showing the review's files; keys go to the file.
        if config.editor.file_tree {
            let mut t = tree::Tree::new(app.tree_entries());
            t.select(app.main_path());
            t.focused = false;
            app.tree = Some(t);
        }
        app.data_version = app.data_version();
        if app.review.requested_at.is_some() {
            app.status = Some((app.request_message(), false));
        }
        Ok(app)
    }

    /// Reads the review's threads and state again, after a change here or elsewhere.
    fn reload(&mut self) {
        self.generation += 1;
        self.placed.borrow_mut().clear();
        let result = (|| -> Result<()> {
            self.review = self.review.reload(&self.conn)?;
            self.threads = review::threads(&self.conn, &self.review.id)?;
            self.unsent = review::unsent(&self.conn, &self.review.id)?;
            self.last_round = review::rounds(&self.conn, &self.review.id)?
                .last()
                .map(|r| r.n);
            Ok(())
        })();
        if let Err(e) = result {
            self.status = Some((format!("{e:#}"), true));
        }
    }

    fn data_version(&self) -> i64 {
        self.conn
            .pragma_query_value(None, "data_version", |r| r.get(0))
            .unwrap_or(0)
    }

    /// Picks up what other processes (an agent's MCP server, the CLI) wrote to the store. Your
    /// view never moves; a new review request shows in the command line.
    fn refresh(&mut self) {
        let v = self.data_version();
        if v == self.data_version {
            return;
        }
        self.data_version = v;
        let before = self.review.clone();
        let unread = |app: &App| app.threads.iter().filter(|t| t.unread()).count();
        let unread_before = unread(self);
        self.reload();
        if self.review.requested_at.is_some() && self.review.requested_at != before.requested_at {
            self.status = Some((self.request_message(), false));
        } else if let Some(s) = &self.review.suggestion
            && before.suggestion.as_ref().map(|b| &b.at) != Some(&s.at)
        {
            let note = match s.note.lines().next().filter(|n| !n.is_empty()) {
                Some(n) => format!(": {n}"),
                None => String::new(),
            };
            self.status = Some((
                format!(
                    "{} suggests {}:{}{note} · space n goes there",
                    s.by, s.path, s.line
                ),
                false,
            ));
        } else if unread(self) > unread_before {
            self.status = Some(("New reply from the agent · ]u goes to it".into(), false));
        }
        if let Some(p) = self.picker.as_mut().filter(|p| dashboard::is_dashboard(p)) {
            match dashboard::items(&self.conn, &self.theme) {
                Ok(items) => p.set_items(items),
                Err(e) => self.status = Some((format!("{e:#}"), true)),
            }
        }
    }

    /// Opens or closes a thread; opening it reads its replies.
    fn toggle_thread(&mut self, i: usize) {
        let t = self.threads[i].clone();
        if !self.expanded.remove(&t.id) {
            self.expanded.insert(t.id.clone());
            self.mark_seen(&t);
        }
    }

    fn mark_seen(&mut self, t: &Thread) {
        if t.unread() {
            if let Err(e) = review::mark_seen(&self.conn, &t.id) {
                self.status = Some((format!("{e:#}"), true));
            }
            self.reload();
        }
    }

    /// `]u` / `[u`: threads with unread agents' replies, in the order of the review's files.
    /// Review-level ones come first; opening one reads it.
    fn goto_unread(&mut self, forward: bool) {
        let order = |path: &str| {
            self.source
                .changes
                .iter()
                .position(|c| c.path == path)
                .unwrap_or(usize::MAX - 1)
        };
        let key = |t: &Thread| match &t.anchor {
            Some(a) => (order(&a.path), a.start as usize - 1),
            None => (0, 0),
        };
        let here = (order(self.main_path()), self.cursor_new_line());
        let mut unread: Vec<(bool, (usize, usize), String)> = self
            .threads
            .iter()
            .filter(|t| t.unread())
            .map(|t| (t.anchor.is_some(), key(t), t.id.clone()))
            .collect();
        unread.sort();
        let next = if forward {
            unread.iter().find(|(lines, k, _)| !lines || *k > here)
        } else {
            unread
                .iter()
                .rev()
                .find(|(lines, k, _)| *lines && *k < here)
        };
        match next.map(|n| n.2.clone()) {
            Some(id) => self.open(Target::Thread(id)),
            None => self.status = Some(("No more unread replies".into(), false)),
        }
    }

    /// The source read again, the way glos found it at the start.
    fn rebuild_source(&self) -> Result<Source> {
        let base = match (&self.source.git, &self.source.paths) {
            (Some(g), _) if self.source.revs.is_some() => {
                let (o, n) = self.source.revs.clone().unwrap();
                Source::revs(&g.worktree, &o, n.as_deref())?
            }
            (Some(g), _) => Source::git(&g.worktree, Some(&g.base))?,
            (None, Some((old, new))) => Source::diff(old, new)?,
            (None, None) => anyhow::bail!("nothing to read again"),
        };
        let mut source = match &self.scope {
            Scope::Base => base,
            Scope::SinceReview(n) => {
                base.since(&review::snapshot(&self.conn, &self.review.id, *n)?)?
            }
            Scope::Commit(i) => {
                let c = self.commits.get(*i).context("no such commit")?;
                base.commit(c.id)?
            }
        };
        match &self.filter {
            Some(Filter::Glob(_, m)) => source.changes.retain(|c| {
                m.is_match(&c.path) || c.old_path.as_ref().is_some_and(|p| m.is_match(p))
            }),
            Some(Filter::Threads) => {
                let open: HashSet<&str> = self
                    .threads
                    .iter()
                    .filter(|t| t.status == "open")
                    .filter_map(|t| t.anchor.as_ref().map(|a| a.path.as_str()))
                    .collect();
                source.changes.retain(|c| open.contains(c.path.as_str()));
            }
            None => {}
        }
        Ok(source)
    }

    /// Changes scope or filter: the changes are found again, and you stay on your file.
    fn rescope(&mut self, scope: Scope, filter: Option<Filter>) {
        let (old_scope, old_filter) = (self.scope.clone(), self.filter.clone());
        self.scope = scope;
        self.filter = filter;
        if let Scope::Commit(_) = self.scope
            && self.commits.is_empty()
        {
            match self.source.commits() {
                Ok(c) if !c.is_empty() => self.commits = c,
                Ok(_) => {
                    self.status = Some(("The branch has no commits since its base".into(), true));
                    (self.scope, self.filter) = (old_scope, old_filter);
                    return;
                }
                Err(e) => {
                    self.status = Some((format!("{e:#}"), true));
                    (self.scope, self.filter) = (old_scope, old_filter);
                    return;
                }
            }
        }
        if let Err(e) = self.rebuild_source() {
            self.status = Some((format!("{e:#}"), true));
            (self.scope, self.filter) = (old_scope, old_filter);
            return;
        }
        self.files_changed();
        // Showing nothing but your file isn't useful: go to the first change in scope.
        if matches!(self.cur, Cur::Context(_)) && !self.source.changes.is_empty() {
            self.open(Target::Change(0));
        }
        self.status = Some((
            match self.source.changes.len() {
                0 => "No changes in this scope".to_string(),
                n => format!(
                    "{}: {n} file{}",
                    self.scope_label(),
                    if n == 1 { "" } else { "s" }
                ),
            },
            false,
        ));
    }

    /// The file tree's entries: each change with its added and removed lines and open threads.
    fn tree_entries(&self) -> Vec<tree::Entry> {
        self.source
            .changes
            .iter()
            .map(|c| {
                let read = |x: &Content| self.source.read(x).ok().flatten().unwrap_or_default();
                let (old, new) = (read(&c.old), read(&c.new));
                let (mut added, mut removed) = (0, 0);
                if !crate::source::is_binary(&old) && !crate::source::is_binary(&new) {
                    for h in crate::diff::hunks(
                        &String::from_utf8_lossy(&old),
                        &String::from_utf8_lossy(&new),
                    ) {
                        added += h.after.len();
                        removed += h.before.len();
                    }
                }
                let threads = self
                    .threads
                    .iter()
                    .filter(|t| {
                        t.status == "open" && t.anchor.as_ref().is_some_and(|a| a.path == c.path)
                    })
                    .count();
                tree::Entry {
                    path: c.path.clone(),
                    added,
                    removed,
                    threads,
                }
            })
            .collect()
    }

    /// Keys while the file tree is focused: `j`/`k` move, `ret` or `l` opens the file (the
    /// tree stays), `esc` goes back to the file, `q` closes the tree.
    fn tree_key(&mut self, key: Key) {
        let Some(t) = &mut self.tree else { return };
        match (key.code, key.char()) {
            (KeyCode::Down, _) | (_, Some('j')) => t.step(true),
            (KeyCode::Up, _) | (_, Some('k')) => t.step(false),
            (KeyCode::Enter, _) | (_, Some('l')) => {
                if let Some(p) = t.selected().map(str::to_string) {
                    t.focused = false;
                    self.open(Target::Path(p));
                }
            }
            (KeyCode::Esc, _) => t.focused = false,
            (_, Some('q')) => {
                self.tree = None;
                self.layout();
            }
            _ => {}
        }
    }

    /// The scope and filter, as the statusline and messages say them.
    fn scope_label(&self) -> String {
        let mut s = match (&self.scope, &self.source.revs) {
            (Scope::Base, Some((o, Some(n)))) => format!("{o} vs {n}"),
            (Scope::Base, Some((o, None))) => format!("{o} vs your files"),
            _ => String::new(),
        };
        if !s.is_empty() {
            match &self.filter {
                Some(Filter::Glob(g, _)) => s += &format!(", files {g}"),
                Some(Filter::Threads) => s += ", files with open threads",
                None => {}
            }
            return s;
        }
        let mut s = match &self.scope {
            Scope::Base if self.source.git.is_some() => "Branch vs base".to_string(),
            Scope::Base => "Old vs new".to_string(),
            Scope::SinceReview(n) => format!("Since round {n}"),
            Scope::Commit(i) => {
                let c = &self.commits[*i];
                let id = c.id.to_string();
                format!(
                    "Commit {}/{} {} {}",
                    i + 1,
                    self.commits.len(),
                    &id[..7],
                    c.summary
                )
            }
        };
        match &self.filter {
            Some(Filter::Glob(g, _)) => s += &format!(", files {g}"),
            Some(Filter::Threads) => s += ", files with open threads",
            None => {}
        }
        s
    }

    /// Files changed on disk: the review's changes are found again, and every loaded file is
    /// read again with its cursor, selections and scroll mapped onto the same code, as Helix
    /// does when a file changes under it. Threads follow their lines.
    fn files_changed(&mut self) {
        self.changed_at = Some(std::time::Instant::now());
        self.generation += 1;
        self.placed.borrow_mut().clear();
        if let Some(a) = self.alt.take() {
            self.sync_base(a);
        }
        let source = match self.rebuild_source() {
            Ok(s) => s,
            Err(e) => {
                self.status = Some((format!("Couldn't read the changes again: {e:#}"), true));
                return;
            }
        };
        let path_of = |app: &App, cur: &Cur| match cur {
            Cur::Change(i) => app.source.changes[*i].path.clone(),
            Cur::Context(p) => p.clone(),
        };
        let cur_path = path_of(self, &self.cur);
        let jumps: Vec<(String, Selection)> = self
            .jumps
            .iter()
            .map(|(c, s)| (path_of(self, c), s.clone()))
            .collect();
        let mut loaded: HashMap<String, File> = HashMap::new();
        for (c, f) in self
            .source
            .changes
            .iter()
            .zip(std::mem::take(&mut self.files))
        {
            if let Some(f) = f {
                loaded.insert(c.path.clone(), f);
            }
        }
        self.source = source;
        self.files = self
            .source
            .changes
            .iter()
            .map(|c| {
                let old = loaded.remove(&c.path)?;
                Some(view::keep_view(
                    old,
                    File::change(c, &self.source, &self.loader),
                ))
            })
            .collect();
        // Files that are no longer changed stay open for context, where they were.
        for (p, old) in loaded {
            if p == cur_path || jumps.iter().any(|(jp, _)| *jp == p) {
                self.context.insert(p, old);
            }
        }
        let paths: Vec<String> = self.context.keys().cloned().collect();
        for p in paths {
            let old = self.context.remove(&p).unwrap();
            let new = File::context(&p, &self.source, &self.loader);
            self.context.insert(p, view::keep_view(old, new));
        }
        let cur_for = |app: &App, p: &str| match app.source.changes.iter().position(|c| c.path == p)
        {
            Some(i) => Cur::Change(i),
            None => Cur::Context(p.to_string()),
        };
        self.cur = cur_for(self, &cur_path);
        match &self.cur {
            Cur::Change(i) => self.load(*i),
            Cur::Context(p) => {
                let p = p.clone();
                self.load_context(&p);
            }
        }
        self.jumps = jumps
            .into_iter()
            .map(|(p, s)| (cur_for(self, &p), s))
            .collect();
        for (c, _) in &self.jumps.clone() {
            match c {
                Cur::Change(i) => self.load(*i),
                Cur::Context(p) => self.load_context(p),
            }
        }
        self.reanchor(false);
        self.ensure_alt();
        if let Some(t) = &self.tree {
            let (focused, selected) = (t.focused, t.selected().map(str::to_string));
            let mut t = tree::Tree::new(self.tree_entries());
            t.focused = focused;
            if let Some(p) = selected {
                t.select(&p);
            }
            self.tree = Some(t);
        }
    }

    /// Places every thread again against the files on disk. Threads whose code is gone are
    /// marked outdated only once `settled`: the files have been quiet for a while.
    fn reanchor(&mut self, settled: bool) {
        let paths: std::collections::BTreeSet<String> = self
            .threads
            .iter()
            .filter_map(|t| t.anchor.as_ref().map(|a| a.path.clone()))
            .collect();
        for p in paths {
            let text = std::fs::read(self.source.root.join(&p))
                .map(|b| String::from_utf8_lossy(&b).into_owned())
                .unwrap_or_default();
            if let Err(e) = review::reanchor(&self.conn, &self.review.id, &p, &text, settled) {
                self.status = Some((format!("{e:#}"), true));
            }
        }
        self.reload();
    }

    fn request_message(&self) -> String {
        let r = &self.review;
        let by = r.requested_by.as_deref().unwrap_or("an agent");
        match r.request_note.as_deref().filter(|n| !n.is_empty()) {
            Some(note) => format!(
                "{by} asks for a review: {}",
                note.lines().next().unwrap_or("")
            ),
            None => format!("{by} asks for a review"),
        }
    }

    /// One frame: the picker's preview is loaded first.
    fn draw(&mut self, f: &mut Frame) {
        self.load_preview();
        self.render(f);
    }
    fn event_loop(&mut self, terminal: &mut DefaultTerminal, mouse: bool) -> Result<()> {
        let size = terminal.size()?;
        self.screen = Rect::new(0, 0, size.width, size.height);
        self.layout();
        let mut first = true;
        while !self.quit {
            terminal.draw(|f| self.draw(f))?;
            if std::mem::take(&mut first) {
                tracing::info!("first screen after {:?}", crate::STARTED.elapsed());
            }
            // About twice a second, look for what other processes wrote.
            if event::poll(Duration::from_millis(500))? {
                match event::read()? {
                    Event::Key(k) if k.kind != KeyEventKind::Release => {
                        if self.key(Key::from(k)) == Some(AppAction::Suspend) {
                            suspend(terminal)?;
                        }
                    }
                    Event::Mouse(m) => self.mouse(m),
                    // Coming back to glos: look at the files again.
                    Event::FocusGained => self.files_changed(),
                    _ => {} // resizes and anything else just redraw
                }
            }
            if std::mem::take(&mut self.run_editor) {
                self.external_editor(terminal, mouse)?;
            }
            if self
                .watch
                .as_ref()
                .is_some_and(|w| w.rx.try_iter().count() > 0)
            {
                self.files_changed();
            }
            if self.changed_at.is_some_and(|t| t.elapsed() >= SETTLE) {
                self.changed_at = None;
                self.reanchor(true);
            }
            self.refresh();
        }
        Ok(())
    }

    /// `:editor`: hands the terminal to `$EDITOR` with the comment box's text, then takes it back.
    fn external_editor(&mut self, terminal: &mut DefaultTerminal, mouse: bool) -> Result<()> {
        let Some(b) = &self.comment_box else {
            return Ok(());
        };
        let text = b.file.text.to_string();
        if mouse {
            let _ = crossterm::execute!(std::io::stdout(), crossterm::event::DisableMouseCapture);
        }
        ratatui::restore();
        let result = comments::external_edit(&comments::editor(), &text);
        crossterm::terminal::enable_raw_mode()?;
        crossterm::execute!(std::io::stdout(), crossterm::terminal::EnterAlternateScreen)?;
        if mouse {
            crossterm::execute!(std::io::stdout(), crossterm::event::EnableMouseCapture)?;
        }
        terminal.clear()?;
        match result {
            Ok(new) => self.set_box_text(&new),
            Err(e) => self.status = Some((format!("{e:#}"), true)),
        }
        Ok(())
    }

    /// Splits the screen: the file view, the comment box when it's open, then the statusline
    /// and the command line.
    fn layout(&mut self) {
        let (w, h) = (self.screen.width, self.screen.height.saturating_sub(2));
        let box_h = if self.comment_box.is_some() {
            BOX_HEIGHT.min(h.saturating_sub(3))
        } else {
            0
        };
        // The file tree takes the left; side by side halves what's left.
        let tree_w = if self.tree.is_some() {
            (w / 3).clamp(16, 40).min(w.saturating_sub(20))
        } else {
            0
        };
        self.tree_area = Rect::new(0, 0, tree_w, h);
        let (x, rest) = (tree_w, w - tree_w);
        self.view = Rect::new(x, 0, rest, h - box_h);
        self.left_view = Rect::default();
        if self.split_left().is_some() {
            let half = rest / 2;
            self.left_view = Rect::new(x, 0, half, h - box_h);
            self.view = Rect::new(x + half, 0, rest - half, h - box_h);
        }
        self.box_area = Rect::new(x, h - box_h, rest, box_h);
    }

    fn load(&mut self, i: usize) {
        if self.files[i].is_none() {
            self.files[i] = Some(File::change(
                &self.source.changes[i],
                &self.source,
                &self.loader,
            ));
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
            Some(Target::Command(_) | Target::Nothing | Target::Review(_)) => {}
            Some(Target::Thread(id)) => {
                if let Some((path, _)) = self.thread_place(&id) {
                    match self.source.changes.iter().position(|c| c.path == path) {
                        Some(i) => self.load(i),
                        None => self.load_context(&path),
                    }
                }
            }
            None => {}
        }
    }

    /// A line thread's file and first line (0-based).
    fn thread_place(&self, id: &str) -> Option<(String, usize)> {
        let a = self.threads.iter().find(|t| t.id == id)?.anchor.as_ref()?;
        Some((a.path.clone(), a.start as usize - 1))
    }

    /// The file keys act on: the comment box while it's open, else the file on screen.
    fn file(&self) -> &File {
        match &self.comment_box {
            Some(b) => &b.file,
            None => self.main_file(),
        }
    }

    // `if let` can't return the borrow here (a borrow checker limit), so this checks first.
    #[allow(clippy::unnecessary_unwrap)]
    fn file_mut(&mut self) -> &mut File {
        if self.comment_box.is_some() {
            return &mut self.comment_box.as_mut().unwrap().file;
        }
        self.main_file_mut()
    }

    /// The file on screen, under the comment box if one is open: in the unified and
    /// side-by-side views, the document built for it.
    fn main_file(&self) -> &File {
        match &self.alt {
            Some(a) if a.cur == self.cur && a.kind == self.view_kind => &a.doc,
            _ => self.base_file(),
        }
    }

    fn main_file_mut(&mut self) -> &mut File {
        let alt = matches!(&self.alt, Some(a) if a.cur == self.cur && a.kind == self.view_kind);
        if alt {
            return &mut self.alt.as_mut().unwrap().doc;
        }
        self.base_file_mut()
    }

    /// The file itself: a change's new side (or its old side, if deleted), or a context file.
    fn base_file(&self) -> &File {
        match &self.cur {
            Cur::Change(i) => self.files[*i].as_ref().expect("the current file is loaded"),
            Cur::Context(p) => &self.context[p],
        }
    }

    fn base_file_mut(&mut self) -> &mut File {
        match &self.cur {
            Cur::Change(i) => self.files[*i].as_mut().expect("the current file is loaded"),
            Cur::Context(p) => self.context.get_mut(p).expect("the current file is loaded"),
        }
    }

    /// The side-by-side view's old side, when it's on screen.
    fn split_left(&self) -> Option<&File> {
        match &self.alt {
            Some(a) if a.cur == self.cur && a.kind == self.view_kind => a.left.as_ref(),
            _ => None,
        }
    }

    /// The cursor's line on the new side: for a removed or folded line in a view, the next
    /// new line.
    fn cursor_new_line(&self) -> usize {
        let f = self.main_file();
        let text = f.text.slice(..);
        let line = text.char_to_line(f.sel.primary().cursor(text));
        f.new_line_near(line).unwrap_or(0)
    }

    /// Builds (or drops) the unified or side-by-side document for the file on screen, keeping
    /// the cursor on the same line of the new side.
    fn ensure_alt(&mut self) {
        let current = matches!(&self.alt, Some(a) if a.cur == self.cur && a.kind == self.view_kind);
        if current {
            return;
        }
        if let Some(a) = self.alt.take() {
            self.sync_base(a);
            if self.view_kind == ViewKind::File {
                self.layout();
                self.scroll_to_cursor(true);
            }
        }
        if self.view_kind == ViewKind::File {
            return;
        }
        let Cur::Change(i) = self.cur.clone() else {
            return; // a context file has no diff
        };
        let base = self.files[i].as_ref().expect("the current file is loaded");
        if base.binary || base.collapsed {
            return;
        }
        let change = &self.source.changes[i];
        let read = |c: &Content| self.source.read(c).ok().flatten().unwrap_or_default();
        let (old_bytes, new_bytes) = (read(&change.old), read(&change.new));
        let path = self.source.root.join(&change.path);
        let old = File::side(&path, &old_bytes, &self.loader);
        let new = File::side(&path, &new_bytes, &self.loader);
        let hunks = crate::diff::hunks(
            &String::from_utf8_lossy(&old_bytes),
            &String::from_utf8_lossy(&new_bytes),
        );
        let (mut doc, left) = match self.view_kind {
            ViewKind::Unified => (
                diffview::unified(&old, &new, &hunks, &self.loader, &self.theme),
                None,
            ),
            _ => {
                let (l, r) = diffview::split(&old, &new, &hunks, &self.loader, &self.theme);
                (r, Some(l))
            }
        };
        // The same place: the base file's cursor line and column.
        let t = base.text.slice(..);
        let cursor = base.sel.primary().cursor(t);
        let line = t.char_to_line(cursor);
        let col = cursor - t.line_to_char(line);
        let dt = doc.text.slice(..);
        let at = doc.line_of(line).min(dt.len_lines() - 1);
        let len = glos_helix::text::line_end_char_index(&dt, at) - dt.line_to_char(at);
        doc.sel = Selection::point(dt.line_to_char(at) + col.min(len)).ensure_invariants(dt);
        self.alt = Some(Alt {
            cur: self.cur.clone(),
            kind: self.view_kind,
            doc,
            left,
        });
        self.layout();
        self.scroll_to_cursor(true);
    }

    /// Leaving a view: its cursor goes back to the file, so the next view opens there.
    fn sync_base(&mut self, a: Alt) {
        let t = a.doc.text.slice(..);
        let line = t.char_to_line(a.doc.sel.primary().cursor(t));
        let Some(n) = a.doc.new_line_near(line) else {
            return;
        };
        let file = match &a.cur {
            Cur::Change(i) => self.files[*i].as_mut(),
            Cur::Context(p) => self.context.get_mut(p),
        };
        if let Some(f) = file {
            let t = f.text.slice(..);
            let n = n.min(t.len_lines() - 1);
            f.sel = Selection::point(t.line_to_char(n)).ensure_invariants(t);
        }
    }

    /// The path of the file on screen, as threads are anchored.
    fn main_path(&self) -> &str {
        match &self.cur {
            Cur::Change(i) => &self.source.changes[*i].path,
            Cur::Context(p) => p,
        }
    }

    /// The commit on the new side of what's shown, as threads record it; `None` for your files.
    fn here_rev(&self) -> Option<String> {
        self.source.new_commit.map(|c| c.to_string())
    }

    /// Where a thread is in what's shown. A thread made on the same new side (your files, or
    /// this commit) is where it was made; one made on another version is placed onto this one
    /// by its code, and isn't shown if that code isn't here. Threads in other files keep their
    /// own lines (they're placed when their file is shown).
    fn shown_anchor(&self, t: &Thread) -> Option<Anchor> {
        let a = t.anchor.as_ref()?;
        let here = self.here_rev();
        if t.on.new_rev == here || a.path != self.main_path() {
            return Some(a.clone());
        }
        let key = (t.id.clone(), self.generation, here);
        if let Some(v) = self.placed.borrow().get(&key) {
            return v.clone();
        }
        let text = self.base_file().text.to_string();
        let v = review::place(&self.conn, &t.id, &text)
            .ok()
            .flatten()
            .map(|(start, end)| Anchor {
                path: a.path.clone(),
                start,
                end,
            });
        self.placed.borrow_mut().insert(key, v.clone());
        v
    }

    /// The threads with their anchors as shown (see [`shown_anchor`]), in the same order.
    fn shown_threads(&self) -> Vec<Thread> {
        self.threads
            .iter()
            .map(|t| Thread {
                anchor: self.shown_anchor(t),
                ..t.clone()
            })
            .collect()
    }

    /// What's compared, for the threads made on it: the commits on each side and a sentence an
    /// agent can read.
    fn diff_context(&self) -> review::DiffContext {
        let s = &self.source;
        let short = |id: &gix::ObjectId| id.to_string()[..7].to_string();
        let old = s.old_commit.as_ref().map(short).unwrap_or_default();
        let new = s.new_commit.as_ref().map(short).unwrap_or_default();
        let diff = match (&self.scope, &s.revs, &s.git, &s.paths) {
            (Scope::Commit(i), _, _, _) => {
                let c = &self.commits[*i];
                let parent = match &s.old_commit {
                    Some(p) => format!("its parent {}", short(p)),
                    None => "nothing (the first commit)".into(),
                };
                format!(
                    "commit {}/{} {new} \"{}\" vs {parent}",
                    i + 1,
                    self.commits.len(),
                    c.summary
                )
            }
            (Scope::SinceReview(n), ..) => {
                format!("your files vs what you reviewed in round {n}")
            }
            (Scope::Base, Some((o, Some(n))), ..) => format!("{o} ({old}) vs {n} ({new})"),
            (Scope::Base, Some((o, None)), ..) => {
                format!("{o} ({old}) vs your files, uncommitted changes included")
            }
            (Scope::Base, None, Some(g), _) => format!(
                "branch {} vs {} (merge base {old}): your files, uncommitted changes included",
                g.branch.as_deref().unwrap_or("(detached HEAD)"),
                g.base
            ),
            (Scope::Base, None, None, Some((o, n))) => {
                format!("{} vs {}", o.display(), n.display())
            }
            (Scope::Base, None, None, None) => "the review".into(),
        };
        review::DiffContext {
            old_rev: s.old_commit.map(|c| c.to_string()),
            new_rev: s.new_commit.map(|c| c.to_string()),
            diff: Some(diff),
        }
    }

    /// The threads of the file on screen, as drawn.
    fn main_threads(&self) -> Threads {
        let here = self.here_rev();
        let t = comments::layout(
            &self.shown_threads(),
            self.main_path(),
            &self.expanded,
            &self.theme,
            here.as_deref(),
        );
        let f = self.main_file();
        if f.rows.is_none() {
            return t;
        }
        // In a view, threads go under the view's line for their last new-side line.
        let mut rows: std::collections::BTreeMap<usize, Vec<comments::ThreadRow>> =
            Default::default();
        for (line, r) in t.rows {
            rows.entry(f.line_of(line)).or_default().extend(r);
        }
        Threads {
            rows,
            marks: t
                .marks
                .into_iter()
                .map(|(s, e, st)| (f.line_of(s), f.line_of(e), st))
                .collect(),
        }
    }

    /// Thread rows as helix-core annotations, so scrolling and the mouse see them; none in the
    /// comment box.
    fn annotations(&self) -> TextAnnotations<'static> {
        let mut a = TextAnnotations::default();
        if self.comment_box.is_none() {
            let lines = ThreadLines::new(&self.main_threads(), self.main_file().text.slice(..));
            a.add_line_annotation(Box::new(lines));
        }
        a
    }

    /// Soft wrap of the focused file: the comment box always wraps; side by side never does,
    /// so the two sides stay level.
    fn wrap(&self) -> bool {
        self.comment_box.is_some() || (self.wrap && self.split_left().is_none())
    }

    /// The focused file's text area: (rows, columns).
    fn text_size(&self) -> (usize, u16) {
        match &self.comment_box {
            Some(_) => (
                self.box_area.height.saturating_sub(2) as usize,
                self.box_area.width.saturating_sub(2),
            ),
            None => {
                let f = self.main_file();
                (
                    self.view.height.saturating_sub(f.notices.len() as u16) as usize,
                    self.view.width.saturating_sub(f.gutter_width() as u16),
                )
            }
        }
    }

    /// Runs `f` with the file on screen focused, even while the comment box is open.
    fn on_main<R>(&mut self, f: impl FnOnce(&mut Self) -> R) -> R {
        let b = self.comment_box.take();
        let r = f(self);
        self.comment_box = b;
        r
    }

    fn open(&mut self, target: Target) {
        match target {
            // Commands run through `choose`; there's nothing to open.
            Target::Command(_) | Target::Nothing => return,
            Target::Review(id) => {
                if id != self.review.id {
                    match dashboard::source_of(&self.conn, &id) {
                        Ok(s) => {
                            self.switch = Some(s);
                            self.quit = true;
                        }
                        Err(e) => self.status = Some((format!("{e:#}"), true)),
                    }
                }
                return;
            }
            Target::Change(i) => {
                let _ = self.push_jump();
                self.load(i);
                self.cur = Cur::Change(i);
                self.ensure_alt();
            }
            Target::Path(p) => {
                // A changed file opens as its change, with its diff.
                if let Some(i) = self.source.changes.iter().position(|c| c.path == p) {
                    return self.open(Target::Change(i));
                }
                let _ = self.push_jump();
                self.load_context(&p);
                self.cur = Cur::Context(p);
            }
            Target::Thread(id) => {
                let Some(t) = self.threads.iter().find(|t| t.id == id).cloned() else {
                    return;
                };
                self.mark_seen(&t);
                match &t.anchor {
                    Some(a) => {
                        self.open(Target::Path(a.path.clone()));
                        self.expanded.insert(t.id.clone());
                        // On what's shown now: it may have been made on another version.
                        match self.shown_anchor(&t) {
                            Some(a) => {
                                self.select_lines(a.start as usize - 1, a.end as usize - 1);
                                self.scroll_to_cursor(true);
                            }
                            None => {
                                let on = t.on.diff.as_deref().unwrap_or("another version");
                                self.status = Some((
                                    format!(
                                        "Its code isn't in what's shown: it was made on {on} \
                                         (space o changes the scope)"
                                    ),
                                    true,
                                ));
                            }
                        }
                    }
                    // A review-level thread has no lines to show: its comment opens instead.
                    None => match t.latest_yours() {
                        Some(c) => self.open_box(Place::Edit(c.id.clone())),
                        None => self.open_box(Place::Reply(t.id.clone())),
                    },
                }
                return;
            }
        }
        self.mode = Mode::Normal;
    }

    /// Selects whole lines `start..=end` (0-based) of the file on screen, as `x` does.
    fn select_lines(&mut self, start: usize, end: usize) {
        let file = self.main_file_mut();
        let (start, end) = (file.line_of(start), file.line_of(end));
        let text = file.text.slice(..);
        let last = text.len_lines() - 1;
        let (from, to) = (
            text.line_to_char(start.min(last)),
            text.line_to_char((end + 1).min(last)),
        );
        file.sel = Selection::single(from, to.max(from)).ensure_invariants(text);
    }

    /// The thread under the cursor in the file on screen: the innermost one holding its line.
    fn thread_here(&self) -> Option<usize> {
        let f = self.main_file();
        let text = f.text.slice(..);
        let line = f.new_line_at(text.char_to_line(f.sel.primary().cursor(text)))? as u32 + 1;
        let path = self.main_path();
        self.shown_threads()
            .iter()
            .enumerate()
            .filter_map(|(i, t)| t.anchor.as_ref().map(|a| (i, a)))
            .filter(|(_, a)| a.path == path && a.start <= line && line <= a.end)
            .max_by_key(|(_, a)| a.start)
            .map(|(i, _)| i)
    }

    /// Where a comment on the selection would go: the primary selection's lines, on the new
    /// side. The final empty line of a file isn't a line to comment on.
    fn can_comment(&self) -> Result<Anchor, &'static str> {
        if let Cur::Change(i) = self.cur
            && self.source.changes[i].new == Content::None
        {
            return Err("Comments go on the new side, and this file was deleted");
        }
        let f = self.main_file();
        if f.binary {
            return Err("A binary file has no lines to comment on");
        }
        if f.collapsed {
            return Err("Expand the file first: ret");
        }
        let text = f.text.slice(..);
        let n = text.len_lines();
        let last = if n > 1 && text.line(n - 1).len_chars() == 0 {
            n - 2
        } else {
            n - 1
        };
        let (s, e) = f.sel.primary().line_range(text);
        // In a view, the selected lines that are on the new side.
        let (s, e) = if f.rows.is_some() {
            let new: Vec<usize> = (s..=e).filter_map(|l| f.new_line_at(l)).collect();
            match (new.iter().min(), new.iter().max()) {
                (Some(a), Some(b)) => (*a, *b),
                _ => return Err("Comments go on the new side: select lines that are on it"),
            }
        } else {
            (s, e.min(last))
        };
        Ok(Anchor {
            path: self.main_path().to_string(),
            start: s.min(e) as u32 + 1,
            end: e as u32 + 1,
        })
    }

    /// Opens the comment box for `place`, with its draft if one was saved.
    fn open_box(&mut self, place: Place) {
        let initial = match &place {
            Place::Edit(c) => self
                .threads
                .iter()
                .flat_map(|t| &t.comments)
                .find(|x| &x.id == c)
                .map(|x| x.body.clone())
                .unwrap_or_default(),
            _ => String::new(),
        };
        let text = match review::draft(&self.conn, &self.review.id, &place.key()) {
            Ok(Some(d)) => {
                self.status = Some(("Draft restored".into(), false));
                d
            }
            Ok(None) => initial,
            Err(e) => {
                self.status = Some((format!("{e:#}"), true));
                initial
            }
        };
        self.comment_box = Some(CommentBox::new(place, &text, &self.loader));
        self.mode = Mode::Normal;
        self.pending.clear();
        self.layout();
        // The commented lines stay in view above the box.
        self.on_main(|app| app.scroll_to_cursor(false));
        self.scroll_to_cursor(false);
    }

    fn set_box_text(&mut self, text: &str) {
        if let Some(b) = &mut self.comment_box {
            b.set_text(text, &self.loader);
            self.save_draft();
            self.scroll_to_cursor(false);
        }
    }

    /// The box's text is saved as you type, so closing, quitting or a crash keeps it.
    fn save_draft(&mut self) {
        let Some(b) = &self.comment_box else { return };
        let body = b.file.text.to_string();
        if let Err(e) = review::save_draft(&self.conn, &self.review.id, &b.place.key(), &body) {
            self.status = Some((format!("Couldn't save the draft: {e:#}"), true));
        }
    }

    /// Keys the comment box handles itself: everything in insert mode, and `ret` (save), `q`
    /// (close), `i` and `a` in normal mode. The rest go through the keymap as usual.
    fn box_key(&mut self, key: Key) -> bool {
        let Some(b) = &mut self.comment_box else {
            return false;
        };
        let ctrl = key.mods.contains(KeyModifiers::CONTROL);
        if b.insert {
            if key.code == KeyCode::Esc || (ctrl && key.code == KeyCode::Char('c')) {
                b.leave_insert();
            } else if !key
                .mods
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                && b.insert_key(key.code, &self.loader)
            {
                self.save_draft();
            }
            self.scroll_to_cursor(false);
            return true;
        }
        match key.code {
            KeyCode::Enter if key.mods.is_empty() => self.save_box(),
            KeyCode::Char('q') if key.mods.is_empty() => self.close_box(),
            KeyCode::Char(c @ ('i' | 'a')) if key.mods.is_empty() => {
                b.enter_insert(c == 'a');
                self.mode = Mode::Normal;
            }
            _ => return false,
        }
        true
    }

    fn close_box(&mut self) {
        if let Some(b) = self.comment_box.take()
            && !b.body().is_empty()
        {
            self.status = Some((
                "Draft kept: it comes back when you reopen the box there".into(),
                false,
            ));
        }
        self.mode = Mode::Normal;
        self.layout();
    }

    /// `ret` in the box: the comment is written to the store, and the box closes.
    fn save_box(&mut self) {
        let file_text = self.base_file().text.to_string();
        let on = self.diff_context();
        let Some(b) = &mut self.comment_box else {
            return;
        };
        let body = b.body();
        let (conn, id) = (&self.conn, &self.review.id);
        let result = match &b.place {
            Place::Edit(_) if body.is_empty() && !b.confirm_delete => {
                b.confirm_delete = true;
                self.status = Some((
                    "Saving it empty deletes the comment: ret again deletes it, q keeps it".into(),
                    true,
                ));
                return;
            }
            Place::Edit(c) if body.is_empty() => review::delete(conn, c).map(|_| "Comment deleted"),
            _ if body.is_empty() => Ok("Nothing to save"),
            Place::Edit(c) => review::edit(conn, c, &body).map(|_| "Comment saved"),
            Place::Reply(t) => review::reply(conn, t, YOU, &body).map(|_| "Reply saved"),
            Place::Review => {
                review::comment_in(conn, id, None, YOU, &body, &on).map(|_| "Comment saved")
            }
            Place::Lines(a) => review::comment_in(conn, id, Some((a, &file_text)), YOU, &body, &on)
                .map(|_| "Comment saved"),
        };
        match result {
            Ok(msg) => {
                let _ = review::save_draft(conn, id, &b.place.key(), "");
                self.comment_box = None;
                self.mode = Mode::Normal;
                self.layout();
                self.reload();
                let unsent = match self.unsent {
                    n if n > 0 && msg != "Nothing to save" => {
                        format!(" · {n} unsent; space s submits")
                    }
                    _ => String::new(),
                };
                self.status = Some((format!("{msg}{unsent}"), false));
            }
            Err(e) => self.status = Some((format!("{e:#}"), true)),
        }
    }

    /// `space s`: every unsent comment goes to the agent as one round, with a verdict.
    fn submit(&mut self, verdict: Verdict) {
        // The round ends: threads whose code is gone are outdated now.
        self.changed_at = None;
        self.reanchor(true);
        let files: Vec<(String, Vec<u8>)> = self
            .source
            .changes
            .iter()
            .filter_map(|c| match self.source.read(&c.new) {
                Ok(Some(bytes)) => Some((c.path.clone(), bytes)),
                _ => None,
            })
            .collect();
        let head = self.source.git.as_ref().map(|g| g.head.to_string());
        match review::submit(
            &self.conn,
            &self.review.id,
            verdict,
            &files,
            head.as_deref(),
        ) {
            Ok(r) => {
                self.reload();
                self.status = Some((
                    format!(
                        "Round {} submitted: {}, {} comment{}. The agent gets it from wait_for_review",
                        r.n,
                        verdict.describe(),
                        r.comments,
                        if r.comments == 1 { "" } else { "s" }
                    ),
                    false,
                ));
            }
            Err(e) => self.status = Some((format!("{e:#}"), true)),
        }
    }

    /// `]d`, `[d`, `]D`, `[D`: to a thread in the file on screen, selecting its lines.
    fn goto_thread(&mut self, forward: bool, edge: bool) {
        let path = self.main_path();
        let mut spans: Vec<(usize, usize)> = self
            .shown_threads()
            .iter()
            .filter_map(|t| t.anchor.clone())
            .filter(|a| a.path == path)
            .map(|a| (a.start as usize - 1, a.end as usize - 1))
            .collect();
        spans.sort();
        spans.dedup();
        let line = self.cursor_new_line();
        let target = match (edge, forward) {
            (true, true) => spans.last(),
            (true, false) => spans.first(),
            (false, true) => spans.iter().find(|s| s.0 > line),
            (false, false) => spans.iter().rev().find(|s| s.0 < line),
        };
        match target.copied() {
            Some((s, e)) => {
                self.select_lines(s, e);
                self.scroll_to_cursor(false);
            }
            None => self.status = Some(("No more threads in this file".into(), false)),
        }
    }

    /// `space d` / `space D`: Helix's diagnostic pickers, over threads.
    fn thread_picker(&self, all: bool) -> Picker {
        let t = &self.theme;
        let path = self.main_path();
        let items = self
            .threads
            .iter()
            .filter(|th| all || th.anchor.as_ref().is_some_and(|a| a.path == path))
            .map(|th| {
                let unsent = th.comments.iter().any(|c| c.unsent());
                let (status, style) = if unsent {
                    ("unsent", t.get("warning"))
                } else {
                    (th.status.as_str(), t.get("info"))
                };
                let place = th
                    .anchor
                    .as_ref()
                    .map_or("review".into(), |a| a.to_string());
                let first = th.comments.first();
                Item {
                    tags: vec![(status.into(), style), (place, t.get("ui.text"))],
                    text: format!(
                        "{}: {}",
                        first.map_or("", |c| c.author.as_str()),
                        first.map_or("", |c| c.body.lines().next().unwrap_or(""))
                    ),
                    target: Target::Thread(th.id.clone()),
                }
            })
            .collect();
        Picker::new(vec!["status", "place", "comment"], items)
    }

    /// Whether a command applies where you are, for the key help panel: thread actions on a
    /// thread, submits with something to send.
    fn applies(&self, name: &str) -> bool {
        let here = || {
            let path = self.main_path();
            self.threads
                .iter()
                .any(|t| t.anchor.as_ref().is_some_and(|a| a.path == path))
        };
        match name {
            "reply" => self.thread_here().is_some(),
            "edit_comment" => self
                .thread_here()
                .is_some_and(|i| self.threads[i].latest_yours().is_some()),
            "submit_changes" | "submit_comment" => self.unsent > 0,
            "comment" => self.can_comment().is_ok(),
            "goto_next_diag" | "goto_prev_diag" | "goto_first_diag" | "goto_last_diag"
            | "diagnostics_picker" => here(),
            "workspace_diagnostics_picker" => !self.threads.is_empty(),
            "view_file" => self.view_kind != ViewKind::File,
            "view_unified" => self.view_kind != ViewKind::Unified,
            "view_split" => self.view_kind != ViewKind::Split,
            "scope_base" => self.scope != Scope::Base,
            "scope_since_review" => {
                self.last_round.is_some() && !matches!(self.scope, Scope::SinceReview(_))
            }
            "scope_commits" => self.source.git.is_some() && !matches!(self.scope, Scope::Commit(_)),
            "goto_next_commit" | "goto_prev_commit" => matches!(self.scope, Scope::Commit(_)),
            _ => true,
        }
    }

    /// Handles one key. Returns the app action that needs the terminal (suspend), if any.
    fn key(&mut self, key: Key) -> Option<AppAction> {
        if let Some(rec) = &mut self.recording {
            rec.push(key);
        }
        if !self.replaying {
            self.status = None;
        }
        // Ctrl-G anywhere: the keys for where you are. Again, or esc, goes back.
        if key.code == KeyCode::Char('g') && key.mods == KeyModifiers::CONTROL {
            match self.palette.take() {
                Some(previous) => self.picker = previous,
                None => self.open_palette(),
            }
            return None;
        }
        if let Some(picker) = &mut self.picker {
            let page = picker.page(picker.areas(self.screen).0);
            match picker.key(key, page) {
                Outcome::Stay => {}
                Outcome::Close => self.picker = self.palette.take().flatten(),
                Outcome::Open(t) => return self.choose(t),
            }
            return None;
        }
        if self.prompt.is_some() {
            self.prompt_key(key);
            return None;
        }
        if self.tree.as_ref().is_some_and(|t| t.focused) {
            self.tree_key(key);
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
                // The motion keeps its own count, so `A-.` repeats it exactly (Helix's
                // `apply_motion`).
                let count = self.count;
                let motion: Motion = match next {
                    NextKey::Char(f, _) => std::rc::Rc::new(move |cx| {
                        cx.count = count;
                        f(cx, ch)
                    }),
                    NextKey::Textobject(around) => std::rc::Rc::new(move |cx| {
                        cx.count = count;
                        commands::select_textobject(cx, around, ch)
                    }),
                };
                self.doc_command(|cx| motion(cx));
                self.last_motion = Some(motion);
            }
            self.count = None;
            return None;
        }
        if self.comment_box.is_some() && self.pending.is_empty() && self.box_key(key) {
            self.count = None;
            return None;
        }
        // A count, as in Helix: digits before a command (a leading 0 isn't one).
        if self.pending.is_empty()
            && let Some(d) = key.char().and_then(|c| c.to_digit(10))
            && (d != 0 || self.count.is_some())
        {
            let n = self.count.map_or(0, NonZeroUsize::get) * 10 + d as usize;
            self.count = NonZeroUsize::new(n.min(100_000_000));
            return None;
        }
        let keymap = if self.mode == Mode::Select {
            &self.keymaps.select
        } else {
            &self.keymaps.normal
        };
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
        let keep: Vec<Key> = sticky
            .map(|n| self.pending[..n.min(self.pending.len())].to_vec())
            .unwrap_or_default();
        self.pending = keep;
        self.execute(command)
    }

    /// An item chosen in a picker: a file or thread opens; a command runs; a key that's only
    /// listed (in the palette for the tree, say) goes back to where you were.
    fn choose(&mut self, t: Target) -> Option<AppAction> {
        let previous = self.palette.take().flatten();
        self.picker = None;
        match t {
            Target::Command(name) => {
                let command = keymap::command(name)?;
                self.pending.clear();
                return self.execute(command);
            }
            Target::Nothing => self.picker = previous,
            t => self.open(t),
        }
        None
    }

    /// `Ctrl-G` and `space ?`: the palette, listing the keys for where you are (Helix's
    /// command palette, made contextual). In the file view, choosing a command runs it.
    fn open_palette(&mut self) {
        let (title, rows) = self.context_keys();
        let style = self.theme.get("ui.text.focus");
        let items = rows
            .into_iter()
            .map(|(keys, doc, name)| Item {
                tags: vec![(keys, style)],
                text: doc.to_string(),
                target: name.map_or(Target::Nothing, Target::Command),
            })
            .collect();
        let mut p = Picker::new(vec!["keys", title], items);
        p.preview = false;
        self.pending.clear();
        self.next_key = None;
        self.count = None;
        self.palette = Some(self.picker.take());
        self.picker = Some(p);
    }

    /// The keys that work where you are: (keys, what they do, the command to run, if any).
    fn context_keys(&self) -> (&'static str, Vec<KeyRow>) {
        let fixed = |rows: &[(&str, &'static str)]| -> Vec<KeyRow> {
            rows.iter()
                .map(|(k, d)| (k.to_string(), *d, None))
                .collect()
        };
        if self.picker.is_some() {
            return ("Picker", fixed(picker::KEYS));
        }
        if let Some(p) = &self.prompt {
            let mut rows = vec![
                ("type", "Write the command or pattern"),
                ("ret", "Run it"),
                ("esc, C-c", "Cancel"),
                ("backspace", "Delete; on an empty line, cancel"),
            ];
            if matches!(p.kind, PromptKind::Command) {
                rows.push(("tab, S-tab", "Complete: the next or previous match"));
            }
            rows.push(("C-g", "These keys"));
            return ("Command line", fixed(&rows));
        }
        if self.tree.as_ref().is_some_and(|t| t.focused) {
            return (
                "File tree",
                fixed(&[
                    ("j, down", "Next file"),
                    ("k, up", "Previous file"),
                    ("ret, l", "Open the file; the tree stays"),
                    ("esc", "Back to the file"),
                    ("q", "Close the tree"),
                    ("space e", "From the file: back to the tree"),
                    ("C-g", "These keys"),
                ]),
            );
        }
        if let Some(b) = &self.comment_box
            && b.insert
        {
            return (
                "Comment box: insert mode",
                fixed(&[
                    ("type", "Write the comment; it's saved as you type"),
                    ("esc, C-c", "Normal mode: then ret saves and q closes"),
                    ("ret", "New line"),
                    ("backspace", "Delete before the cursor"),
                    ("del", "Delete under the cursor"),
                    ("left, right, up, down", "Move"),
                    ("home, end", "Line start, line end"),
                    ("C-g", "These keys"),
                ]),
            );
        }
        let mut rows = vec![];
        let title = if self.comment_box.is_some() {
            rows.extend(fixed(&[
                ("ret", "Save the comment"),
                ("q", "Close the box; the draft is kept"),
                ("i, a", "Insert before, after the selection"),
                (":editor", "Edit the comment in $EDITOR"),
            ]));
            "Comment box: normal mode"
        } else if self.mode == Mode::Select {
            "Select mode"
        } else {
            "Normal mode"
        };
        let keymap = if self.mode == Mode::Select {
            &self.keymaps.select
        } else {
            &self.keymaps.normal
        };
        self.walk_keys(keymap, "", &mut rows);
        rows.push(("C-g".into(), "These keys", None));
        (title, rows)
    }

    /// Every command under `menu` that applies here, with its full key sequence; keys bound to
    /// the same command are joined.
    fn walk_keys(&self, menu: &keymap::Menu, prefix: &str, out: &mut Vec<KeyRow>) {
        let start = out.len();
        for (key, node) in &menu.keys {
            let keys = format!("{prefix}{key}");
            match node {
                Node::Cmd(c) => {
                    let boxed = self.comment_box.is_some();
                    // The box's own keys win over the keymap's, and some actions wait for it.
                    if boxed
                        && (matches!(keys.as_str(), "q" | "ret" | "i" | "a")
                            || matches!(c.action, Action::App(a) if blocked_in_box(a)))
                    {
                        continue;
                    }
                    if !self.applies(c.name) {
                        continue;
                    }
                    match out[start..].iter_mut().find(|r| r.2 == Some(c.name)) {
                        Some(r) => r.0 = format!("{}, {keys}", r.0),
                        None => out.push((keys, c.doc, Some(c.name))),
                    }
                }
                Node::Menu(m) => self.walk_keys(m, &format!("{keys} "), out),
            }
        }
    }

    /// Runs a command, from its keys or from the palette.
    fn execute(&mut self, command: &'static keymap::Command) -> Option<AppAction> {
        let result = match command.action {
            Action::Doc(f) => {
                // As Helix: these jumps go on the jumplist, and these motions repeat with A-.
                if self.comment_box.is_none()
                    && matches!(
                        command.name,
                        "goto_file_start" | "goto_last_line" | "goto_line"
                    )
                {
                    let _ = self.push_jump();
                }
                if matches!(
                    command.name,
                    "goto_next_change"
                        | "goto_prev_change"
                        | "goto_next_paragraph"
                        | "goto_prev_paragraph"
                ) {
                    let count = self.count;
                    self.last_motion = Some(std::rc::Rc::new(move |cx: &mut Ctx| {
                        cx.count = count;
                        f(cx)
                    }));
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
        let Some(prompt) = &mut self.prompt else {
            return;
        };
        match key.code {
            KeyCode::Esc => match self.prompt.take() {
                Some(Prompt {
                    kind: PromptKind::Search { origin, top, .. },
                    ..
                }) => {
                    let file = self.file_mut();
                    file.sel = origin;
                    file.top = top;
                }
                Some(Prompt {
                    kind: PromptKind::Regex { origin, .. },
                    ..
                }) => self.file_mut().sel = origin,
                _ => {}
            },
            KeyCode::Char('c') if key.mods.contains(KeyModifiers::CONTROL) => {
                self.prompt_key(Key {
                    code: KeyCode::Esc,
                    mods: KeyModifiers::NONE,
                });
            }
            KeyCode::Enter => {
                let Some(Prompt { kind, line, .. }) = self.prompt.take() else {
                    return;
                };
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
                        if commands::search_regex(self.last_search.as_deref().unwrap_or(""))
                            .is_none()
                        {
                            self.status = Some(("Invalid regex".into(), true));
                        }
                    }
                }
            }
            KeyCode::Tab | KeyCode::BackTab if matches!(prompt.kind, PromptKind::Command) => {
                // Helix's completion: each tab shows the next match for what was typed.
                let typed = match &prompt.completion {
                    Some((t, _)) => t.clone(),
                    None => prompt.line.clone(),
                };
                let (start, items) = completions(&typed);
                if items.is_empty() {
                    return;
                }
                let n = items.len();
                let back = key.code == KeyCode::BackTab || key.mods.contains(KeyModifiers::SHIFT);
                let i = match (&prompt.completion, back) {
                    (None, true) => n - 1,
                    (None, false) => 0,
                    (Some((_, i)), true) => (i + n - 1) % n,
                    (Some((_, i)), false) => (i + 1) % n,
                };
                prompt.line = format!("{}{}", &typed[..start], items[i].0);
                prompt.completion = Some((typed, i));
            }
            KeyCode::Backspace => {
                prompt.completion = None;
                if prompt.line.pop().is_none() {
                    return self.prompt_key(Key {
                        code: KeyCode::Esc,
                        mods: KeyModifiers::NONE,
                    });
                }
                self.search_preview();
            }
            KeyCode::Char(c)
                if !key
                    .mods
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                prompt.completion = None;
                prompt.line.push(c);
                self.search_preview();
            }
            _ => {}
        }
    }

    /// As you type a search, the selection jumps to the first match from where you started; a
    /// regex prompt applies to the selections it started from.
    fn search_preview(&mut self) {
        if let Some(Prompt {
            kind: PromptKind::Regex { action, origin },
            line,
            ..
        }) = &self.prompt
        {
            let (action, origin) = (*action, origin.clone());
            let Some(regex) = commands::search_regex(line) else {
                return;
            };
            self.file_mut().sel = origin;
            self.doc_command(|cx| commands::regex_selections(cx, action, &regex));
            self.status = None;
            return;
        }
        let Some(Prompt {
            kind:
                PromptKind::Search {
                    forward,
                    origin,
                    top,
                },
            line,
            ..
        }) = &self.prompt
        else {
            return;
        };
        let (forward, origin, top) = (*forward, origin.clone(), *top);
        let Some(regex) = commands::search_regex(line) else {
            return;
        };
        let file = self.file_mut();
        file.sel = origin;
        file.top = top;
        self.doc_command(|cx| commands::search(cx, &regex, forward, false));
    }

    fn app_command(&mut self, action: AppAction) -> Option<AppAction> {
        use AppAction as A;
        if self.comment_box.is_some() && blocked_in_box(action) {
            self.status = Some((
                "Close the comment box first: q in normal mode (the draft is kept)".into(),
                true,
            ));
            return None;
        }
        match action {
            A::Comment => match self.can_comment() {
                Ok(a) => self.open_box(Place::Lines(a)),
                Err(e) => self.status = Some((e.into(), true)),
            },
            A::ReviewComment => self.open_box(Place::Review),
            A::Reply => match self.thread_here() {
                Some(i) => self.open_box(Place::Reply(self.threads[i].id.clone())),
                None => self.status = Some(("No thread here to reply to".into(), true)),
            },
            A::EditComment => {
                match self
                    .thread_here()
                    .and_then(|i| self.threads[i].latest_yours())
                    .map(|c| c.id.clone())
                {
                    Some(c) => self.open_box(Place::Edit(c)),
                    None => self.status = Some(("No comment of yours here".into(), true)),
                }
            }
            A::Submit(v) => self.submit(v),
            A::ThreadPicker { all } => self.picker = Some(self.thread_picker(all)),
            A::Dashboard => match dashboard::picker(&self.conn, &self.theme) {
                Ok(p) => self.picker = Some(p),
                Err(e) => self.status = Some((format!("{e:#}"), true)),
            },
            A::NextThread(forward) => self.goto_thread(forward, false),
            A::EdgeThread(last) => self.goto_thread(last, true),
            A::NextUnread(forward) => self.goto_unread(forward),
            A::ToggleResolved => match self.thread_here() {
                Some(i) => {
                    let t = &self.threads[i];
                    let (status, msg) = if t.status == "open" {
                        ("resolved", "Thread resolved")
                    } else {
                        ("open", "Thread reopened")
                    };
                    let id = t.id.clone();
                    match review::set_status(&self.conn, &id, status, YOU, "") {
                        Ok(()) => self.status = Some((msg.into(), false)),
                        Err(e) => self.status = Some((format!("{e:#}"), true)),
                    }
                    self.reload();
                }
                None => self.status = Some(("No thread here".into(), true)),
            },
            A::View(kind) => {
                self.view_kind = kind;
                self.ensure_alt();
                if kind != ViewKind::File && self.split_left().is_none() && self.alt.is_none() {
                    self.status = Some((
                        format!(
                            "This file has no diff to show {}; showing it whole",
                            kind.name()
                        ),
                        false,
                    ));
                }
            }
            A::Scope(which) => {
                let scope = match which {
                    keymap::ScopeKey::Base => Some(Scope::Base),
                    keymap::ScopeKey::SinceReview => self.last_round.map(Scope::SinceReview),
                    keymap::ScopeKey::Commits => Some(Scope::Commit(0)),
                };
                match scope {
                    Some(s) => self.rescope(s, self.filter.clone()),
                    None => {
                        self.status =
                            Some(("Nothing submitted yet, so no last review".into(), true))
                    }
                }
            }
            A::NextCommit(forward) => match self.scope {
                Scope::Commit(i) => {
                    let n = self.commits.len();
                    let step = self.count.map_or(1, NonZeroUsize::get);
                    let j = if forward {
                        (i + step).min(n - 1)
                    } else {
                        i.saturating_sub(step)
                    };
                    if j == i {
                        self.status = Some((
                            if forward {
                                "The last commit"
                            } else {
                                "The first commit"
                            }
                            .into(),
                            false,
                        ));
                    } else {
                        self.rescope(Scope::Commit(j), self.filter.clone());
                    }
                }
                _ => self.status = Some(("Not in the per-commit scope: space o c".into(), true)),
            },
            A::FilterPrompt => {
                let mut p = Prompt::new(PromptKind::Command);
                p.line = "filter ".into();
                self.prompt = Some(p);
            }
            A::CommandPalette => self.open_palette(),
            A::FileTree => match &mut self.tree {
                Some(t) if !t.focused => t.focused = true,
                Some(_) => self.tree = None,
                None => {
                    let mut t = tree::Tree::new(self.tree_entries());
                    t.select(self.main_path());
                    self.tree = Some(t);
                }
            },
            A::GotoSuggestion => match self.review.suggestion.clone() {
                Some(s) => {
                    self.open(Target::Path(s.path.clone()));
                    let line = s.line as usize - 1;
                    self.select_lines(line, line);
                    self.scroll_to_cursor(true);
                }
                None => self.status = Some(("No agent has suggested a line".into(), false)),
            },
            AppAction::SelectMode => self.mode = Mode::Select,
            AppAction::NormalMode => self.mode = Mode::Normal,
            AppAction::CommandMode => self.prompt = Some(Prompt::new(PromptKind::Command)),
            AppAction::Search(forward) => {
                let file = self.file();
                let kind = PromptKind::Search {
                    forward,
                    origin: file.sel.clone(),
                    top: file.top,
                };
                self.prompt = Some(Prompt::new(kind));
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
                if n == 0 {
                    self.status = Some(("No changed files left".into(), false));
                    return None;
                }
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
                // On a thread, ret opens or closes it.
                if let Some(i) = self.thread_here() {
                    self.toggle_thread(i);
                    return None;
                }
                let file = self.base_file_mut();
                let marker = "generated file, collapsed; enter expands";
                if file.notices.iter().any(|n| n == marker) || file.collapsed {
                    file.collapsed = !file.collapsed;
                }
                self.ensure_alt();
            }
            AppAction::Yank | AppAction::YankToClipboard { .. } => {
                let file = self.file();
                let text = file.text.slice(..);
                let mut values: Vec<String> =
                    file.sel.fragments(text).map(|f| f.into_owned()).collect();
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
                    Ok(()) => (
                        format!(
                            "yanked {n} selection{} to register {register}",
                            if n == 1 { "" } else { "s" }
                        ),
                        false,
                    ),
                    Err(e) => (format!("{e:#}"), true),
                });
                self.mode = Mode::Normal; // as Helix, yanking leaves select mode
            }
            AppAction::RegexPrompt(action) => {
                let kind = PromptKind::Regex {
                    action,
                    origin: self.file().sel.clone(),
                };
                self.prompt = Some(Prompt::new(kind));
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
                let (cur, sel) = target?;
                self.cur = cur;
                if let Cur::Change(i) = self.cur {
                    self.load(i);
                }
                self.ensure_alt();
                let file = self.file_mut();
                file.sel = sel.ensure_invariants(file.text.slice(..));
                self.scroll_to_cursor(true);
            }
            AppAction::RepeatLastMotion => {
                // A count repeats the motion that many times, each with its own count.
                if let Some(m) = self.last_motion.clone() {
                    for _ in 0..self.count.take().map_or(1, NonZeroUsize::get) {
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
                    self.status = Some((
                        "Cannot replay from register [@] because already replaying".into(),
                        true,
                    ));
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
        let entry = (self.cur.clone(), self.main_file().sel.clone());
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
        let here = (self.cur.clone(), self.main_file().sel.clone());
        if self.jumps.get(self.jump)? == &here {
            self.jump = self.jump.checked_sub(1)?;
        }
        self.jumps.get(self.jump).cloned()
    }

    /// Click to move the cursor, drag to select, wheel to scroll; in the picker, the wheel moves
    /// and a click opens.
    fn mouse(&mut self, m: MouseEvent) {
        if let Some(picker) = &mut self.picker {
            let (list, _) = picker.areas(self.screen);
            match m.kind {
                MouseEventKind::ScrollDown => {
                    picker.key(
                        Key {
                            code: KeyCode::Down,
                            mods: KeyModifiers::NONE,
                        },
                        1,
                    );
                }
                MouseEventKind::ScrollUp => {
                    picker.key(
                        Key {
                            code: KeyCode::Up,
                            mods: KeyModifiers::NONE,
                        },
                        1,
                    );
                }
                MouseEventKind::Down(MouseButton::Left) => {
                    if let Some(t) = picker.target_at(list, m.column, m.row) {
                        self.choose(t);
                    }
                }
                _ => {}
            }
            return;
        }
        if self.prompt.is_some() || self.comment_box.is_some() {
            return;
        }
        // A click in the file tree opens that file.
        if let Some(t) = &self.tree
            && m.column < self.tree_area.right()
        {
            if m.kind == MouseEventKind::Down(MouseButton::Left)
                && let Some(p) = t.path_at(self.tree_area, m.row).map(str::to_string)
            {
                self.open(Target::Path(p.clone()));
                if let Some(t) = &mut self.tree {
                    t.select(&p);
                    t.focused = false;
                }
            }
            return;
        }
        // A click on a thread opens or closes it.
        if m.kind == MouseEventKind::Down(MouseButton::Left)
            && let Some(&(_, i)) = self.thread_hits.iter().find(|(y, _)| *y == m.row)
        {
            self.toggle_thread(i);
            return;
        }
        match m.kind {
            MouseEventKind::ScrollDown | MouseEventKind::ScrollUp => {
                // Helix's editor.scroll-lines: 3, and the cursor stays in view.
                let down = m.kind == MouseEventKind::ScrollDown;
                self.doc_command(|cx| commands::scroll_lines(cx, 3, down));
            }
            MouseEventKind::Down(MouseButton::Left) | MouseEventKind::Drag(MouseButton::Left) => {
                let Some(pos) = self.pos_at(m.column, m.row) else {
                    return;
                };
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
        let file = self.main_file();
        let header = file.notices.len() as u16;
        let gutter = file.gutter_width() as u16;
        if y < self.view.y + header
            || y >= self.view.bottom()
            || x < self.view.x + gutter
            || file.collapsed
            || file.binary
        {
            return None;
        }
        let text = file.text.slice(..);
        let fmt = view::text_format(file, self.view.width.saturating_sub(gutter), self.wrap);
        let left = if fmt.soft_wrap { 0 } else { file.left };
        let row = (y - self.view.y - header) as isize;
        let col = left + (x - self.view.x - gutter) as usize;
        let anchor = text.line_to_char(file.top.min(text.len_lines() - 1));
        let (pos, _) = char_idx_at_visual_offset(text, anchor, row, col, &fmt, &self.annotations());
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
                Item {
                    tags: vec![(tag.into(), style)],
                    text,
                    target: Target::Change(i),
                }
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
        let walker = ignore::WalkBuilder::new(root)
            .hidden(true)
            .require_git(false)
            .build();
        for entry in walker.flatten().take(100_000) {
            if entry
                .file_type()
                .is_some_and(|t| t.is_file() || t.is_symlink())
                && let Ok(rel) = entry.path().strip_prefix(root)
            {
                let p = rel.to_string_lossy().into_owned();
                // A changed file previews and opens as its change.
                let target = match self.source.changes.iter().position(|c| c.path == p) {
                    Some(i) => Target::Change(i),
                    None => Target::Path(p.clone()),
                };
                items.push(Item {
                    tags: vec![],
                    text: p,
                    target,
                });
            }
        }
        items.sort_by(|a, b| a.text.cmp(&b.text));
        Picker::new(vec!["path"], items)
    }

    /// `:` commands (see [`TYPABLE`]), and `:<n>` to go to line n.
    fn typed_command(&mut self, line: &str) {
        let (name, args) = line.split_once(' ').unwrap_or((line, ""));
        match name {
            "" => {}
            "q" | "quit" | "qa" if self.unsent > 0 && !self.reminded => {
                self.reminded = true;
                let n = self.unsent;
                self.status = Some((
                    format!(
                        "{n} comment{} not sent to the agent: space s submits; :q again quits and keeps {}",
                        if n == 1 { "" } else { "s" },
                        if n == 1 { "it" } else { "them" }
                    ),
                    true,
                ));
            }
            "q" | "quit" | "q!" | "quit!" | "qa" | "qa!" => self.quit = true,
            "comment" => _ = self.app_command(AppAction::Comment),
            "dashboard" => _ = self.app_command(AppAction::Dashboard),
            "close" => match review::close(&self.conn, &self.review.id) {
                Ok(()) => self.quit = true,
                Err(e) => self.status = Some((format!("{e:#}"), true)),
            },
            "review-comment" => _ = self.app_command(AppAction::ReviewComment),
            "reply" => _ = self.app_command(AppAction::Reply),
            "edit-comment" => _ = self.app_command(AppAction::EditComment),
            "submit" => match Verdict::parse(args.trim()) {
                Some(v) => _ = self.app_command(AppAction::Submit(v)),
                None => {
                    self.status = Some((
                        "Submit how? :submit approve, :submit changes or :submit comment".into(),
                        true,
                    ))
                }
            },
            "view" => match args.trim() {
                "file" => _ = self.app_command(AppAction::View(ViewKind::File)),
                "unified" => _ = self.app_command(AppAction::View(ViewKind::Unified)),
                "split" => _ = self.app_command(AppAction::View(ViewKind::Split)),
                _ => self.status = Some(("View how? :view file, unified or split".into(), true)),
            },
            "scope" => {
                let key = match args.trim() {
                    "base" => Some(keymap::ScopeKey::Base),
                    "since" => Some(keymap::ScopeKey::SinceReview),
                    "commits" => Some(keymap::ScopeKey::Commits),
                    _ => None,
                };
                match key {
                    Some(k) => _ = self.app_command(AppAction::Scope(k)),
                    None => {
                        self.status = Some(("Scope? :scope base, since or commits".into(), true))
                    }
                }
            }
            "filter" => {
                let arg = args.trim();
                let filter = match arg {
                    "" => Ok(None),
                    "threads" => Ok(Some(Filter::Threads)),
                    g => globset::Glob::new(g)
                        .map(|glob| Some(Filter::Glob(g.to_string(), glob.compile_matcher()))),
                };
                match filter {
                    Ok(f) => {
                        let scope = self.scope.clone();
                        self.rescope(scope, f);
                    }
                    Err(e) => self.status = Some((format!("Bad glob: {e}"), true)),
                }
            }
            "editor" => match self.comment_box {
                Some(_) => self.run_editor = true,
                None => {
                    self.status = Some(("No comment box is open: space c opens one".into(), true))
                }
            },
            "toggle" | "toggle-option" => match args.trim() {
                "soft-wrap" | "soft-wrap.enable" => {
                    self.wrap = !self.wrap;
                    self.status = Some((
                        format!("'soft-wrap.enable' is now set to {}", self.wrap),
                        false,
                    ));
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
        let (count, extend, wrap) = (self.count, self.mode == Mode::Select, self.wrap());
        let (height, width) = self.text_size();
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
            visual_col: view::visual_col,
        };
        f(&mut cx);
        let (sel, top, left, status, center, keep_top) = (
            cx.sel.ensure_invariants(file.text.slice(..)),
            cx.top,
            cx.left,
            cx.status,
            cx.center,
            cx.keep_top,
        );
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
        let (height, width) = self.text_size();
        let wrap = self.wrap();
        let annotations = self.annotations();
        let file = self.file_mut();
        let fmt = view::text_format(file, width, wrap);
        let text = file.text.slice(..);
        let cursor = file.sel.primary().cursor(text);
        let line = text.char_to_line(cursor);
        let so = commands::SCROLLOFF.min(height.saturating_sub(1) / 2);
        // The cursor's row below the top of the view, if it's within `height` rows.
        let row_of = |top: usize| {
            if line < top {
                return None;
            }
            visual_offset_from_anchor(
                text,
                text.line_to_char(top),
                cursor,
                &fmt,
                &annotations,
                height,
            )
            .ok()
            .map(|(p, _)| p.row)
        };
        // Lines from `top` the cursor would sit at, if `top` were the first line shown.
        let row = row_of(file.top);
        let above = line < file.top || (file.top > 0 && row.is_some_and(|r| r < so));
        let below = !above && row.is_none_or(|r| r + so >= height);
        let outside = line < file.top || row.is_none_or(|r| r >= height);
        if center && outside {
            // The cursor half way down: back up from its line while it stays above the middle.
            file.top = line;
            while file.top > 0 && row_of(file.top - 1).is_some_and(|r| r <= height / 2) {
                file.top -= 1;
            }
        } else if above {
            // Up to the cursor's line, then back `so` rows more.
            file.top = line;
            while file.top > 0 && row_of(file.top - 1).is_some_and(|r| r <= so) {
                file.top -= 1;
            }
        } else if below {
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
        self.layout();
        let current = self.main_path().to_string();
        let tree_area = self.tree_area;
        if let Some(t) = &mut self.tree {
            t.render(buf, tree_area, &current, &self.theme);
        }
        let threads = self.main_threads();
        if let Some(left) = self.split_left() {
            // The old side follows the new side's scroll; its thread rows are blank so the
            // sides stay level.
            let right = self.main_file();
            let mut blank = Threads::default();
            for (line, rows) in &threads.rows {
                let pad = rows
                    .iter()
                    .map(|r| comments::ThreadRow {
                        text: String::new(),
                        style: Style::default(),
                        thread: r.thread,
                    })
                    .collect();
                blank.rows.insert(*line, pad);
            }
            let none = Style::default();
            view::draw(
                buf,
                self.left_view,
                left,
                right.top,
                right.left,
                false,
                &self.loader,
                &self.theme,
                Some((none, none, none, none)),
                &blank,
            );
        }
        let file = self.main_file();
        // Under the comment box, the commented lines keep normal mode's colours.
        let styles = if self.comment_box.is_some() {
            self.selection_styles_in("normal")
        } else {
            self.selection_styles()
        };
        self.thread_hits = view::draw(
            buf,
            self.view,
            file,
            file.top,
            file.left,
            self.wrap && self.split_left().is_none(),
            &self.loader,
            &self.theme,
            Some(styles),
            &threads,
        );
        self.render_box(buf);
        self.render_statusline(buf, Rect::new(0, area.height - 2, area.width, 1));
        self.render_command_line(buf, Rect::new(0, area.height - 1, area.width, 1));
        self.render_completion(buf, Rect::new(0, 0, area.width, area.height - 2));
        self.render_picker(buf, area);
        self.render_help(f, area);
    }

    /// The comment box, docked under the file view, with its place as the title.
    fn render_box(&self, buf: &mut Buffer) {
        let Some(b) = &self.comment_box else { return };
        let t = &self.theme;
        let popup = t.get("ui.popup");
        let hint = if b.insert {
            " esc: normal mode "
        } else {
            " ret saves · q closes · :editor "
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .title(format!(" {} ", clean(&b.place.title())))
            .title_bottom(hint)
            .style(popup)
            .border_style(popup);
        let inner = block.inner(self.box_area);
        Clear.render(self.box_area, buf);
        block.render(self.box_area, buf);
        view::draw(
            buf,
            inner,
            &b.file,
            b.file.top,
            b.file.left,
            true,
            &self.loader,
            t,
            Some(self.selection_styles()),
            &Threads::default(),
        );
    }

    /// `:` completions above the command line, as Helix lists them, with what each does.
    fn render_completion(&self, buf: &mut Buffer, area: Rect) {
        let Some(Prompt {
            kind: PromptKind::Command,
            line,
            completion,
        }) = &self.prompt
        else {
            return;
        };
        let typed = completion
            .as_ref()
            .map_or(line.as_str(), |(t, _)| t.as_str());
        let (_, items) = completions(typed);
        if items.is_empty() || area.height == 0 {
            return;
        }
        let t = &self.theme;
        let rows = items.len().min(area.height as usize).min(10);
        let name_w = items.iter().map(|(n, _)| n.width()).max().unwrap_or(0);
        let selected = completion.as_ref().map(|(_, i)| *i);
        // The selected one stays in view.
        let first = selected.map_or(0, |i| (i + 1).saturating_sub(rows));
        let y0 = area.bottom() - rows as u16;
        for (row, (i, (name, doc))) in items.iter().enumerate().skip(first).take(rows).enumerate() {
            let style = if Some(i) == selected {
                t.get("ui.menu.selected")
            } else {
                t.get("ui.menu")
            };
            let r = Rect::new(area.x, y0 + row as u16, area.width, 1);
            buf.set_style(r, style);
            buf.set_stringn(
                r.x + 1,
                r.y,
                format!("{name:name_w$}  {doc}"),
                area.width.saturating_sub(2) as usize,
                style,
            );
        }
    }

    fn render_picker(&mut self, buf: &mut Buffer, area: Rect) {
        let Some(picker) = &mut self.picker else {
            return;
        };
        let (list, preview) = picker.areas(area);
        picker.render(buf, list, &self.theme);
        let Some(preview) = preview else { return };
        let inner = picker::preview_frame(buf, preview, &self.theme);
        let selected = picker.selected().cloned();
        let at_path = |p: &str| match self.source.changes.iter().position(|c| c.path == p) {
            Some(i) => self.files[i].as_ref(),
            None => self.context.get(p),
        };
        let (file, line) = match &selected {
            Some(Target::Change(i)) => (self.files[*i].as_ref(), None),
            Some(Target::Path(p)) => (self.context.get(p), None),
            Some(Target::Thread(id)) => match self.thread_place(id) {
                Some((p, line)) => (at_path(&p), Some(line)),
                None => (None, None),
            },
            Some(Target::Command(_) | Target::Nothing | Target::Review(_)) | None => (None, None),
        };
        if let Some(file) = file {
            // Centre the thread, or the first change, as Helix centres a preview on its range.
            let line = line.or(file.hunks.first().map(|h| h.after.start as usize));
            let top = line.map_or(0, |l| l.saturating_sub(inner.height as usize / 3));
            view::draw(
                buf,
                inner,
                file,
                top,
                0,
                self.wrap,
                &self.loader,
                &self.theme,
                None,
                &Threads::default(),
            );
        }
    }

    /// (selection, primary selection, cursor, primary cursor), as Helix picks them per mode.
    fn selection_styles(&self) -> view::SelectionStyles {
        let mode = match (&self.comment_box, self.mode) {
            (Some(b), _) if b.insert => "insert",
            (_, Mode::Select) => "select",
            _ => "normal",
        };
        self.selection_styles_in(mode)
    }

    fn selection_styles_in(&self, mode: &str) -> view::SelectionStyles {
        let t = &self.theme;
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

    fn render_statusline(&self, buf: &mut Buffer, area: Rect) {
        let base = self.theme.get("ui.statusline");
        buf.set_style(area, base);
        let file = self.file();
        let (path, which) = match &self.cur {
            Cur::Change(i) => (
                self.source.changes[*i].path.as_str(),
                format!("{}/{} files", i + 1, self.files.len()),
            ),
            Cur::Context(p) => (p.as_str(), "context".to_string()),
        };
        let mode = match (&self.comment_box, self.mode) {
            (Some(b), _) if b.insert => " INS ",
            (_, Mode::Select) => " SEL ",
            _ => " NOR ",
        };
        let mut left = format!("{mode} {}", clean(path));
        if self.scope != Scope::Base || self.filter.is_some() || self.source.revs.is_some() {
            left += &format!(" · {}", clean(&self.scope_label()));
        }
        if self.view_kind != ViewKind::File {
            left += &format!(" · {}", self.view_kind.name());
        }
        if let Some(b) = &self.comment_box {
            left += &format!(" · {}", clean(&b.place.title()));
        }
        let which = match self.unsent {
            0 => which,
            n => format!("{n} unsent  {which}"),
        };
        let unread = self.threads.iter().filter(|t| t.unread()).count();
        let which = match unread {
            0 => which,
            n => format!("{n} unread  {which}"),
        };
        buf.set_stringn(area.x, area.y, &left, area.width as usize, base);

        let text = file.text.slice(..);
        let cursor = file.sel.primary().cursor(text);
        let line = text.char_to_line(cursor);
        let col = cursor - text.line_to_char(line);
        // In a view, the position is the file's: its new-side line.
        let line = file.new_line_near(line).unwrap_or(line);
        let sels = file.sel.len();
        let right = format!(
            " {which}  {sels} sel{}  {}:{} ",
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
            let prefix = match p.kind {
                PromptKind::Command => ":",
                PromptKind::Search { .. } => "search:",
                PromptKind::Regex { action, .. } => action.prompt(),
            };
            let shown = format!("{prefix}{}", clean(&p.line));
            buf.set_stringn(
                area.x,
                area.y,
                &shown,
                area.width as usize,
                t.get("ui.text"),
            );
            let x = area.x + shown.width() as u16;
            if x < area.right() {
                buf[(x, area.y)].set_style(t.get("ui.cursor.primary"));
            }
            return;
        }
        if let Some((msg, error)) = &self.status {
            let style = if *error {
                t.get("error")
            } else {
                t.get("ui.text")
            };
            buf.set_stringn(area.x, area.y, clean(msg), area.width as usize, style);
        }
        // Pending keys and count at the bottom right, as Helix shows them.
        let mut pending: String = self.count.map(|c| c.to_string()).unwrap_or_default();
        for k in &self.pending {
            pending += &k.to_string();
        }
        if !pending.is_empty() {
            let w = pending.width() as u16;
            buf.set_string(
                area.x + area.width.saturating_sub(w + 1),
                area.y,
                &pending,
                t.get("ui.text"),
            );
        }
    }

    /// The key help panel (Helix's infobox): what can follow the keys typed so far.
    fn render_help(&self, f: &mut Frame, area: Rect) {
        let (title, rows): (&str, Vec<(String, &str)>) = if let Some(next) = self.next_key {
            match next {
                NextKey::Textobject(around) => (
                    if around {
                        "Match around"
                    } else {
                        "Match inside"
                    },
                    commands::TEXTOBJECT_HELP
                        .iter()
                        .map(|(k, d)| (k.to_string(), *d))
                        .collect(),
                ),
                NextKey::Char(_, doc) => (doc, vec![]),
            }
        } else if !self.pending.is_empty() {
            let keymap = if self.mode == Mode::Select {
                &self.keymaps.select
            } else {
                &self.keymaps.normal
            };
            let mut menu = keymap;
            for k in &self.pending {
                if let Some(Node::Menu(m)) = menu.get(*k) {
                    menu = m;
                }
            }
            let rows = menu
                .help()
                .into_iter()
                .filter(|(_, _, name)| name.is_none_or(|n| self.applies(n)))
                .map(|(k, d, _)| (k, d))
                .collect();
            (menu.name, rows)
        } else {
            return;
        };
        if rows.is_empty() {
            return;
        }
        let key_w = rows.iter().map(|(k, _)| k.width()).max().unwrap_or(0);
        let lines: Vec<String> = rows
            .iter()
            .map(|(k, d)| format!("{k:key_w$}  {d}"))
            .collect();
        let width = (lines
            .iter()
            .map(|l| l.width())
            .max()
            .unwrap_or(0)
            .max(title.width())
            + 4) as u16;
        let width = width.min(area.width);
        let height = (lines.len() as u16 + 2).min(area.height.saturating_sub(2));
        let rect = Rect::new(
            area.width - width,
            area.height.saturating_sub(2 + height),
            width,
            height,
        );
        let t = &self.theme;
        let popup = t.get("ui.popup.info");
        f.render_widget(Clear, rect);
        f.render_widget(
            Block::default()
                .borders(Borders::ALL)
                .title(title)
                .style(popup)
                .border_style(popup),
            rect,
        );
        let text_style: Style = popup.patch(t.get("ui.text.info"));
        for (i, l) in lines
            .iter()
            .enumerate()
            .take(height.saturating_sub(2) as usize)
        {
            f.buffer_mut().set_stringn(
                rect.x + 2,
                rect.y + 1 + i as u16,
                l,
                width.saturating_sub(3) as usize,
                text_style,
            );
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
mod tests;
