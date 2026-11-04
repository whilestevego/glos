//! Helix's keymap: a trie of keys per mode, with menus (`g`, `m`, `[`, …) that open the key
//! help panel. Built from Helix's default keymap at 25.07.1, keeping only what applies to glos.

use std::fmt;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::commands::{self as c, Ctx};
use crate::review::Verdict;

/// What a command does when it runs.
#[derive(Clone, Copy)]
pub enum Action {
    /// Acts on the current file's selection.
    Doc(fn(&mut Ctx)),
    /// Waits for one more character first (`f`, `t`, …).
    NextChar(fn(&mut Ctx, char)),
    /// `mi` / `ma`: waits for the text object's character; `true` for around.
    Textobject(bool),
    App(AppAction),
}

/// Commands that act on glos itself rather than one file's selection.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AppAction {
    SelectMode,
    NormalMode,
    CommandMode,
    NextFile,
    PrevFile,
    Suspend,
    /// `/` and `?`: `true` searches forward.
    Search(bool),
    SearchNext(bool),
    /// `*` and `A-*`: `true` detects word boundaries.
    SearchSelection(bool),
    FilePicker,
    ChangedFilePicker,
    ToggleCollapse,
    /// `y`: the selections to glos's own register.
    Yank,
    /// `space y` / `space Y`: all selections, or only the primary, to the system clipboard.
    YankToClipboard {
        main_only: bool,
    },
    /// `s`, `S`, `K`, `A-K`: a regex prompt that acts on the selections.
    RegexPrompt(super::commands::RegexAction),
    JumpForward,
    JumpBackward,
    SaveSelection,
    RepeatLastMotion,
    RecordMacro,
    ReplayMacro,
    /// `space c`: comment on the selected lines.
    Comment,
    /// `space C`: a review-level comment.
    ReviewComment,
    Reply,
    EditComment,
    Submit(crate::review::Verdict),
    /// `space d` / `space D`: threads in this file, or the whole review.
    ThreadPicker {
        all: bool,
    },
    /// `]d` / `[d`: `true` goes forward.
    NextThread(bool),
    /// `]D` / `[D`: `true` goes to the last.
    EdgeThread(bool),
    /// `]u` / `[u`: the next or previous thread with an agent's reply you haven't read.
    NextUnread(bool),
    /// `space x`: resolve the thread under the cursor, or reopen it.
    ToggleResolved,
    /// `space n`: the line an agent suggested.
    GotoSuggestion,
    /// `space v` then `f`, `u` or `s`.
    View(super::ViewKind),
    /// `space o` then `b`, `r` or `c`.
    Scope(ScopeKey),
    /// `]C` / `[C`: the next or previous commit, in the per-commit scope.
    NextCommit(bool),
    /// `space o f`: the `:filter` prompt.
    FilterPrompt,
    /// `space e`: open, focus or close the file tree.
    FileTree,
    /// `space ?` (and `C-g` anywhere): the palette of keys for where you are.
    CommandPalette,
    /// `space l`: every open review.
    Dashboard,
}

/// Scopes as keys choose them; "since last review" becomes the latest round.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ScopeKey {
    Base,
    SinceReview,
    Commits,
}

pub struct Command {
    pub name: &'static str,
    pub doc: &'static str,
    pub action: Action,
}

macro_rules! commands {
    ($($name:ident, $doc:literal, $action:expr;)*) => {
        /// Every command, by its Helix name and with Helix's description.
        pub static COMMANDS: &[Command] = &[
            $(Command { name: stringify!($name), doc: $doc, action: $action },)*
        ];
    };
}

use Action::{App, Doc, NextChar};
commands! {
    move_char_left, "Move left", Doc(c::move_char_left);
    move_char_right, "Move right", Doc(c::move_char_right);
    move_visual_line_up, "Move up", Doc(c::move_visual_line_up);
    move_visual_line_down, "Move down", Doc(c::move_visual_line_down);
    move_next_word_start, "Move to start of next word", Doc(c::move_next_word_start);
    move_prev_word_start, "Move to start of previous word", Doc(c::move_prev_word_start);
    move_next_word_end, "Move to end of next word", Doc(c::move_next_word_end);
    move_next_long_word_start, "Move to start of next long word", Doc(c::move_next_long_word_start);
    move_prev_long_word_start, "Move to start of previous long word", Doc(c::move_prev_long_word_start);
    move_next_long_word_end, "Move to end of next long word", Doc(c::move_next_long_word_end);
    find_till_char, "Move till next occurrence of char", NextChar(c::find_till_char);
    find_next_char, "Move to next occurrence of char", NextChar(c::find_next_char);
    till_prev_char, "Move till previous occurrence of char", NextChar(c::till_prev_char);
    find_prev_char, "Move to previous occurrence of char", NextChar(c::find_prev_char);
    page_up, "Move page up", Doc(c::page_up);
    page_down, "Move page down", Doc(c::page_down);
    page_cursor_half_up, "Move page and cursor half up", Doc(c::page_cursor_half_up);
    page_cursor_half_down, "Move page and cursor half down", Doc(c::page_cursor_half_down);
    select_all, "Select whole document", Doc(c::select_all);
    extend_line_below, "Select current line, if already selected, extend to next line", Doc(c::extend_line_below);
    extend_to_line_bounds, "Extend selection to line bounds", Doc(c::extend_to_line_bounds);
    collapse_selection, "Collapse selection into single cursor", Doc(c::collapse_selection);
    flip_selections, "Flip selection cursor and anchor", Doc(c::flip_selections);
    keep_primary_selection, "Keep primary selection", Doc(c::keep_primary_selection);
    goto_file_start, "Goto line number <n> else file start", Doc(c::goto_file_start);
    goto_last_line, "Goto last line", Doc(c::goto_last_line);
    goto_line, "Goto line", Doc(c::goto_line);
    goto_line_start, "Goto line start", Doc(c::goto_line_start);
    goto_line_end, "Goto line end", Doc(c::goto_line_end);
    goto_first_nonwhitespace, "Goto first non-blank in line", Doc(c::goto_first_nonwhitespace);
    goto_window_top, "Goto window top", Doc(c::goto_window_top);
    goto_window_center, "Goto window center", Doc(c::goto_window_center);
    goto_window_bottom, "Goto window bottom", Doc(c::goto_window_bottom);
    goto_next_change, "Goto next change", Doc(c::goto_next_change);
    goto_prev_change, "Goto previous change", Doc(c::goto_prev_change);
    goto_first_change, "Goto first change", Doc(c::goto_first_change);
    goto_last_change, "Goto last change", Doc(c::goto_last_change);
    match_brackets, "Goto matching bracket", Doc(c::match_brackets);
    select_textobject_around, "Select around object", Action::Textobject(true);
    select_textobject_inner, "Select inside object", Action::Textobject(false);
    // Helix calls changed files' neighbours buffers; in glos they're the review's files.
    goto_next_buffer, "Goto next changed file", App(AppAction::NextFile);
    goto_previous_buffer, "Goto previous changed file", App(AppAction::PrevFile);
    select_mode, "Enter selection extend mode", App(AppAction::SelectMode);
    normal_mode, "Enter normal mode", App(AppAction::NormalMode);
    exit_select_mode, "Exit selection mode", App(AppAction::NormalMode);
    command_mode, "Enter command mode", App(AppAction::CommandMode);
    search, "Search for regex pattern", App(AppAction::Search(true));
    rsearch, "Reverse search for regex pattern", App(AppAction::Search(false));
    search_next, "Select next search match", App(AppAction::SearchNext(true));
    search_prev, "Select previous search match", App(AppAction::SearchNext(false));
    search_selection_detect_word_boundaries, "Use current selection as the search pattern, automatically wrapping with `\\b` on word boundaries", App(AppAction::SearchSelection(true));
    search_selection, "Use current selection as search pattern", App(AppAction::SearchSelection(false));
    file_picker, "Open file picker", App(AppAction::FilePicker);
    changed_file_picker, "Open changed file picker", App(AppAction::ChangedFilePicker);
    toggle_collapse, "Expand or collapse the file", App(AppAction::ToggleCollapse);
    yank, "Yank selection", App(AppAction::Yank);
    move_line_up, "Move up", Doc(c::move_line_up);
    move_line_down, "Move down", Doc(c::move_line_down);
    goto_column, "Goto column", Doc(c::goto_column);
    goto_next_paragraph, "Goto next paragraph", Doc(c::goto_next_paragraph);
    goto_prev_paragraph, "Goto previous paragraph", Doc(c::goto_prev_paragraph);
    copy_selection_on_next_line, "Copy selection on next line", Doc(c::copy_selection_on_next_line);
    copy_selection_on_prev_line, "Copy selection on previous line", Doc(c::copy_selection_on_prev_line);
    trim_selections, "Trim whitespace from selections", Doc(c::trim_selections);
    rotate_selections_forward, "Rotate selections forward", Doc(c::rotate_selections_forward);
    rotate_selections_backward, "Rotate selections backward", Doc(c::rotate_selections_backward);
    merge_selections, "Merge selections", Doc(c::merge_selections);
    merge_consecutive_selections, "Merge consecutive selections", Doc(c::merge_consecutive_selections);
    ensure_selections_forward, "Ensure all selections face forward", Doc(c::ensure_selections_forward);
    shrink_to_line_bounds, "Shrink selection to line bounds", Doc(c::shrink_to_line_bounds);
    remove_primary_selection, "Remove primary selection", Doc(c::remove_primary_selection);
    split_selection_on_newline, "Split selection on newlines", Doc(c::split_selection_on_newline);
    select_regex, "Select all regex matches inside selections", App(AppAction::RegexPrompt(c::RegexAction::Select));
    split_selection, "Split selections on regex matches", App(AppAction::RegexPrompt(c::RegexAction::Split));
    keep_selections, "Keep selections matching regex", App(AppAction::RegexPrompt(c::RegexAction::Keep));
    remove_selections, "Remove selections matching regex", App(AppAction::RegexPrompt(c::RegexAction::Remove));
    align_view_center, "Align view center", Doc(c::align_view_center);
    align_view_top, "Align view top", Doc(c::align_view_top);
    align_view_bottom, "Align view bottom", Doc(c::align_view_bottom);
    align_view_middle, "Align view middle", Doc(c::align_view_middle);
    scroll_up, "Scroll view up", Doc(c::scroll_up);
    scroll_down, "Scroll view down", Doc(c::scroll_down);
    jump_forward, "Jump forward on jumplist", App(AppAction::JumpForward);
    jump_backward, "Jump backward on jumplist", App(AppAction::JumpBackward);
    save_selection, "Save current selection to jumplist", App(AppAction::SaveSelection);
    repeat_last_motion, "Repeat last motion", App(AppAction::RepeatLastMotion);
    record_macro, "Record macro", App(AppAction::RecordMacro);
    replay_macro, "Replay macro", App(AppAction::ReplayMacro);
    yank_to_clipboard, "Yank selections to clipboard", App(AppAction::YankToClipboard { main_only: false });
    yank_main_selection_to_clipboard, "Yank main selection to clipboard", App(AppAction::YankToClipboard { main_only: true });
    suspend, "Suspend and return to shell", App(AppAction::Suspend);
    // glos's review actions, on the keys Helix leaves to LSP and editing.
    comment, "Comment on the selected lines", App(AppAction::Comment);
    review_comment, "Comment on the whole review", App(AppAction::ReviewComment);
    reply, "Reply to the thread under the cursor", App(AppAction::Reply);
    edit_comment, "Edit your latest comment on the thread under the cursor", App(AppAction::EditComment);
    submit_approve, "Approve", App(AppAction::Submit(Verdict::Approved));
    submit_changes, "Request changes", App(AppAction::Submit(Verdict::Changes));
    submit_comment, "Send comments without a verdict", App(AppAction::Submit(Verdict::Comment));
    // Threads are glos's diagnostics.
    diagnostics_picker, "Open thread picker", App(AppAction::ThreadPicker { all: false });
    workspace_diagnostics_picker, "Open thread picker for the whole review", App(AppAction::ThreadPicker { all: true });
    goto_next_diag, "Goto next thread", App(AppAction::NextThread(true));
    goto_prev_diag, "Goto previous thread", App(AppAction::NextThread(false));
    goto_last_diag, "Goto last thread", App(AppAction::EdgeThread(true));
    goto_first_diag, "Goto first thread", App(AppAction::EdgeThread(false));
    goto_next_unread, "Goto next unread reply", App(AppAction::NextUnread(true));
    goto_prev_unread, "Goto previous unread reply", App(AppAction::NextUnread(false));
    toggle_resolved, "Resolve or reopen the thread under the cursor", App(AppAction::ToggleResolved);
    goto_suggestion, "Goto the line the agent suggested", App(AppAction::GotoSuggestion);
    view_file, "The whole file, changes in the gutter", App(AppAction::View(super::ViewKind::File));
    view_unified, "Unified: changes with lines around them", App(AppAction::View(super::ViewKind::Unified));
    view_split, "Side by side: old and new", App(AppAction::View(super::ViewKind::Split));
    scope_base, "Branch vs base (or old vs new)", App(AppAction::Scope(ScopeKey::Base));
    scope_since_review, "Since your last review", App(AppAction::Scope(ScopeKey::SinceReview));
    scope_commits, "One commit at a time", App(AppAction::Scope(ScopeKey::Commits));
    filter_files, "Filter files by glob, or to those with threads", App(AppAction::FilterPrompt);
    goto_next_commit, "Goto next commit", App(AppAction::NextCommit(true));
    goto_prev_commit, "Goto previous commit", App(AppAction::NextCommit(false));
    file_explorer, "Open the file tree", App(AppAction::FileTree);
    command_palette, "Open command palette", App(AppAction::CommandPalette);
    dashboard, "Open the dashboard: every open review", App(AppAction::Dashboard);
}

pub fn command(name: &str) -> Option<&'static Command> {
    COMMANDS.iter().find(|c| c.name == name)
}

/// A key as Helix writes it: `h`, `G`, `C-d`, `A-;`, `space`, `ret`, `esc`, `left`…
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Key {
    pub code: KeyCode,
    pub mods: KeyModifiers,
}

impl From<KeyEvent> for Key {
    fn from(e: KeyEvent) -> Key {
        let mut mods =
            e.modifiers & (KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SHIFT);
        // Shift is already in the character (`G`, `%`), as Helix normalises it.
        if matches!(e.code, KeyCode::Char(_)) {
            mods.remove(KeyModifiers::SHIFT);
        }
        Key { code: e.code, mods }
    }
}

const NAMED: &[(&str, KeyCode)] = &[
    ("space", KeyCode::Char(' ')),
    ("ret", KeyCode::Enter),
    ("esc", KeyCode::Esc),
    ("tab", KeyCode::Tab),
    ("backspace", KeyCode::Backspace),
    ("del", KeyCode::Delete),
    ("left", KeyCode::Left),
    ("right", KeyCode::Right),
    ("up", KeyCode::Up),
    ("down", KeyCode::Down),
    ("home", KeyCode::Home),
    ("end", KeyCode::End),
    ("pageup", KeyCode::PageUp),
    ("pagedown", KeyCode::PageDown),
    ("minus", KeyCode::Char('-')),
];

impl Key {
    pub fn parse(s: &str) -> Option<Key> {
        let mut mods = KeyModifiers::NONE;
        let mut rest = s;
        loop {
            if let Some(r) = rest.strip_prefix("C-") {
                mods |= KeyModifiers::CONTROL;
                rest = r;
            } else if let Some(r) = rest.strip_prefix("A-") {
                mods |= KeyModifiers::ALT;
                rest = r;
            } else if let Some(r) = rest.strip_prefix("S-") {
                mods |= KeyModifiers::SHIFT;
                rest = r;
            } else {
                break;
            }
        }
        let code = match NAMED.iter().find(|(n, _)| *n == rest) {
            Some((_, c)) => *c,
            None => {
                let mut chars = rest.chars();
                let ch = chars.next()?;
                if chars.next().is_some() {
                    return None;
                }
                KeyCode::Char(ch)
            }
        };
        Some(Key { code, mods })
    }

    pub fn char(&self) -> Option<char> {
        match self.code {
            KeyCode::Char(c) if self.mods.is_empty() => Some(c),
            _ => None,
        }
    }
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if self.mods.contains(KeyModifiers::CONTROL) {
            f.write_str("C-")?;
        }
        if self.mods.contains(KeyModifiers::ALT) {
            f.write_str("A-")?;
        }
        if self.mods.contains(KeyModifiers::SHIFT) {
            f.write_str("S-")?;
        }
        match NAMED.iter().find(|(_, c)| *c == self.code) {
            Some((n, _)) => f.write_str(n),
            None => match self.code {
                KeyCode::Char(c) => write!(f, "{c}"),
                other => write!(f, "{other:?}"),
            },
        }
    }
}

pub enum Node {
    Cmd(&'static Command),
    Menu(Menu),
}

pub struct Menu {
    pub name: &'static str,
    pub keys: Vec<(Key, Node)>,
    /// A sticky menu stays open after a command, until `esc` (Helix's `Z`).
    pub sticky: bool,
}

impl Menu {
    pub fn get(&self, key: Key) -> Option<&Node> {
        self.keys.iter().find(|(k, _)| *k == key).map(|(_, n)| n)
    }

    /// The key help panel's rows: keys bound to the same thing are joined (`h, left`). Each
    /// row has its command's name (none for a menu), so rows that don't apply can be left out.
    pub fn help(&self) -> Vec<(String, &'static str, Option<&'static str>)> {
        let mut rows: Vec<(String, &'static str, Option<&'static str>, *const ())> = vec![];
        for (key, node) in &self.keys {
            let (doc, name, id) = match node {
                Node::Cmd(c) => (c.doc, Some(c.name), *c as *const Command as *const ()),
                Node::Menu(m) => (m.name, None, m as *const Menu as *const ()),
            };
            match rows.iter_mut().find(|r| r.3 == id) {
                Some(r) => r.0 = format!("{}, {key}", r.0),
                None => rows.push((key.to_string(), doc, name, id)),
            }
        }
        rows.into_iter().map(|(k, d, n, _)| (k, d, n)).collect()
    }
}

enum Def {
    Cmd(&'static str, &'static str),
    Menu(&'static str, &'static str, bool, Vec<Def>),
}

fn cmd(keys: &'static str, name: &'static str) -> Def {
    Def::Cmd(keys, name)
}

fn menu(key: &'static str, name: &'static str, defs: Vec<Def>) -> Def {
    Def::Menu(key, name, false, defs)
}

fn sticky_menu(key: &'static str, name: &'static str, defs: Vec<Def>) -> Def {
    Def::Menu(key, name, true, defs)
}

fn build(name: &'static str, sticky: bool, defs: Vec<Def>) -> Menu {
    let mut keys = vec![];
    for def in defs {
        match def {
            Def::Cmd(ks, c) => {
                let command = command(c).unwrap_or_else(|| panic!("no command {c}"));
                // `|` separates aliases, so the `|` key itself is written alone.
                let aliases: Vec<&str> = if ks == "|" {
                    vec!["|"]
                } else {
                    ks.split('|').collect()
                };
                for k in aliases {
                    keys.push((
                        Key::parse(k).unwrap_or_else(|| panic!("bad key {k}")),
                        Node::Cmd(command),
                    ));
                }
            }
            Def::Menu(k, name, sticky, defs) => {
                keys.push((
                    Key::parse(k).unwrap(),
                    Node::Menu(build(name, sticky, defs)),
                ));
            }
        }
    }
    Menu { name, keys, sticky }
}

/// Helix's default normal-mode keys that apply to a read-only review (keymap/default.rs).
fn normal_defs() -> Vec<Def> {
    vec![
        cmd("h|left", "move_char_left"),
        cmd("j|down", "move_visual_line_down"),
        cmd("k|up", "move_visual_line_up"),
        cmd("l|right", "move_char_right"),
        cmd("t", "find_till_char"),
        cmd("f", "find_next_char"),
        cmd("T", "till_prev_char"),
        cmd("F", "find_prev_char"),
        cmd("home", "goto_line_start"),
        cmd("end", "goto_line_end"),
        cmd("w", "move_next_word_start"),
        cmd("b", "move_prev_word_start"),
        cmd("e", "move_next_word_end"),
        cmd("W", "move_next_long_word_start"),
        cmd("B", "move_prev_long_word_start"),
        cmd("E", "move_next_long_word_end"),
        cmd("v", "select_mode"),
        cmd("G", "goto_line"),
        menu(
            "g",
            "Goto",
            vec![
                cmd("g", "goto_file_start"),
                cmd("e", "goto_last_line"),
                cmd("h", "goto_line_start"),
                cmd("l", "goto_line_end"),
                cmd("s", "goto_first_nonwhitespace"),
                cmd("t", "goto_window_top"),
                cmd("c", "goto_window_center"),
                cmd("b", "goto_window_bottom"),
                cmd("n", "goto_next_buffer"),
                cmd("p", "goto_previous_buffer"),
                cmd("k", "move_line_up"),
                cmd("j", "move_line_down"),
                cmd("|", "goto_column"),
            ],
        ),
        cmd(":", "command_mode"),
        cmd(";", "collapse_selection"),
        cmd("A-;", "flip_selections"),
        cmd("%", "select_all"),
        cmd("x", "extend_line_below"),
        cmd("X", "extend_to_line_bounds"),
        menu(
            "m",
            "Match",
            vec![
                cmd("m", "match_brackets"),
                cmd("a", "select_textobject_around"),
                cmd("i", "select_textobject_inner"),
            ],
        ),
        menu(
            "[",
            "Left bracket",
            vec![
                cmd("d", "goto_prev_diag"),
                cmd("D", "goto_first_diag"),
                cmd("u", "goto_prev_unread"),
                cmd("C", "goto_prev_commit"),
                cmd("g", "goto_prev_change"),
                cmd("G", "goto_first_change"),
                cmd("p", "goto_prev_paragraph"),
            ],
        ),
        menu(
            "]",
            "Right bracket",
            vec![
                cmd("d", "goto_next_diag"),
                cmd("D", "goto_last_diag"),
                cmd("u", "goto_next_unread"),
                cmd("C", "goto_next_commit"),
                cmd("g", "goto_next_change"),
                cmd("G", "goto_last_change"),
                cmd("p", "goto_next_paragraph"),
            ],
        ),
        cmd("C", "copy_selection_on_next_line"),
        cmd("A-C", "copy_selection_on_prev_line"),
        cmd("s", "select_regex"),
        cmd("A-s", "split_selection_on_newline"),
        cmd("A-minus", "merge_selections"),
        cmd("A-_", "merge_consecutive_selections"),
        cmd("S", "split_selection"),
        cmd("A-x", "shrink_to_line_bounds"),
        cmd("K", "keep_selections"),
        cmd("A-K", "remove_selections"),
        cmd("A-,", "remove_primary_selection"),
        cmd("_", "trim_selections"),
        cmd("(", "rotate_selections_backward"),
        cmd(")", "rotate_selections_forward"),
        cmd("A-:", "ensure_selections_forward"),
        cmd("A-.", "repeat_last_motion"),
        cmd("Q", "record_macro"),
        cmd("q", "replay_macro"),
        cmd("esc", "normal_mode"),
        cmd("C-i|tab", "jump_forward"),
        cmd("C-o", "jump_backward"),
        cmd("C-s", "save_selection"),
        menu("z", "View", view_defs()),
        sticky_menu("Z", "View", view_defs()),
        cmd(",", "keep_primary_selection"),
        cmd("/", "search"),
        cmd("?", "rsearch"),
        cmd("n", "search_next"),
        cmd("N", "search_prev"),
        cmd("*", "search_selection_detect_word_boundaries"),
        cmd("A-*", "search_selection"),
        // glos: expand or collapse a generated file.
        cmd("ret", "toggle_collapse"),
        cmd("y", "yank"),
        menu(
            "space",
            "Space",
            vec![
                cmd("f", "file_picker"),
                cmd("g", "changed_file_picker"),
                cmd("d", "diagnostics_picker"),
                cmd("D", "workspace_diagnostics_picker"),
                cmd("y", "yank_to_clipboard"),
                cmd("Y", "yank_main_selection_to_clipboard"),
                // Review actions, where Helix has LSP and editing commands.
                cmd("c", "comment"),
                cmd("C", "review_comment"),
                cmd("r", "reply"),
                cmd("m", "edit_comment"),
                cmd("x", "toggle_resolved"),
                cmd("n", "goto_suggestion"),
                cmd("e", "file_explorer"),
                cmd("l", "dashboard"),
                cmd("?", "command_palette"),
                menu(
                    "v",
                    "View",
                    vec![
                        cmd("f", "view_file"),
                        cmd("u", "view_unified"),
                        cmd("s", "view_split"),
                    ],
                ),
                menu(
                    "o",
                    "Scope",
                    vec![
                        cmd("b", "scope_base"),
                        cmd("r", "scope_since_review"),
                        cmd("c", "scope_commits"),
                        cmd("f", "filter_files"),
                    ],
                ),
                menu(
                    "s",
                    "Submit",
                    vec![
                        cmd("a", "submit_approve"),
                        cmd("r", "submit_changes"),
                        cmd("c", "submit_comment"),
                    ],
                ),
            ],
        ),
        cmd("C-b|pageup", "page_up"),
        cmd("C-f|pagedown", "page_down"),
        cmd("C-u", "page_cursor_half_up"),
        cmd("C-d", "page_cursor_half_down"),
        cmd("C-z", "suspend"),
    ]
}

/// Helix's view menu, under `z` and (sticky) `Z`.
fn view_defs() -> Vec<Def> {
    vec![
        cmd("z|c", "align_view_center"),
        cmd("t", "align_view_top"),
        cmd("b", "align_view_bottom"),
        cmd("m", "align_view_middle"),
        cmd("k|up", "scroll_up"),
        cmd("j|down", "scroll_down"),
        cmd("C-b|pageup", "page_up"),
        cmd("C-f|pagedown", "page_down"),
        cmd("C-u|backspace", "page_cursor_half_up"),
        cmd("C-d|space", "page_cursor_half_down"),
        cmd("/", "search"),
        cmd("?", "rsearch"),
        cmd("n", "search_next"),
        cmd("N", "search_prev"),
    ]
}

pub struct Keymaps {
    pub normal: Menu,
    pub select: Menu,
}

impl Keymaps {
    pub fn default() -> Keymaps {
        let mut select = normal_defs();
        // Select mode is normal mode with motions extending; `v` and `esc` leave it.
        select.retain(|d| !matches!(d, Def::Cmd("v" | "esc", _)));
        select.push(cmd("v", "normal_mode"));
        select.push(cmd("esc", "exit_select_mode"));
        Keymaps {
            normal: build("Normal mode", false, normal_defs()),
            select: build("Select mode", false, select),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_round_trip() {
        for k in [
            "h", "G", "C-d", "A-;", "space", "ret", "esc", "pageup", "%", "minus",
        ] {
            assert_eq!(Key::parse(k).unwrap().to_string(), k);
        }
        let shifted = Key::from(KeyEvent::new(KeyCode::Char('G'), KeyModifiers::SHIFT));
        assert_eq!(shifted, Key::parse("G").unwrap());
    }

    #[test]
    fn help_joins_aliases() {
        let k = Keymaps::default();
        let help = k.normal.help();
        assert!(help.contains(&("h, left".to_string(), "Move left", Some("move_char_left"))));
        assert!(help.contains(&("g".to_string(), "Goto", None)));
        let Some(Node::Menu(g)) = k.normal.get(Key::parse("g").unwrap()) else {
            panic!()
        };
        assert_eq!(g.name, "Goto");
        assert!(
            matches!(k.select.get(Key::parse("v").unwrap()), Some(Node::Cmd(c)) if c.name == "normal_mode")
        );
    }

    fn all_menus<'a>(m: &'a Menu, out: &mut Vec<&'a Menu>) {
        out.push(m);
        for (_, n) in &m.keys {
            if let Node::Menu(sub) = n {
                all_menus(sub, out);
            }
        }
    }

    #[test]
    fn no_key_is_bound_twice_in_a_menu() {
        let k = Keymaps::default();
        for root in [&k.normal, &k.select] {
            let mut menus = vec![];
            all_menus(root, &mut menus);
            for m in menus {
                let mut keys: Vec<_> = m.keys.iter().map(|(k, _)| k.to_string()).collect();
                keys.sort();
                let before = keys.len();
                keys.dedup();
                assert_eq!(before, keys.len(), "a key is bound twice in {}", m.name);
            }
        }
    }

    #[test]
    fn every_command_is_bound_and_described() {
        let k = Keymaps::default();
        let mut bound = std::collections::HashSet::new();
        let mut menus = vec![];
        all_menus(&k.normal, &mut menus);
        all_menus(&k.select, &mut menus);
        for m in menus {
            for (_, n) in &m.keys {
                if let Node::Cmd(c) = n {
                    bound.insert(c.name);
                }
            }
        }
        for c in COMMANDS {
            assert!(!c.doc.is_empty(), "{} has no description", c.name);
            assert!(bound.contains(c.name), "{} isn't bound to any key", c.name);
        }
    }

    #[test]
    fn select_mode_differs_only_where_helix_does() {
        let k = Keymaps::default();
        let get = |m: &Menu, key: &str| match m.get(Key::parse(key).unwrap()) {
            Some(Node::Cmd(c)) => Some(c.name),
            Some(Node::Menu(m)) => Some(m.name),
            None => None,
        };
        assert_eq!(get(&k.normal, "v"), Some("select_mode"));
        assert_eq!(get(&k.select, "v"), Some("normal_mode"));
        assert_eq!(get(&k.normal, "esc"), Some("normal_mode"));
        assert_eq!(get(&k.select, "esc"), Some("exit_select_mode"));
        for key in ["h", "w", "x", "g", "space", "z", "/", "n", "f"] {
            assert_eq!(get(&k.normal, key), get(&k.select, key), "{key}");
        }
        let Some(Node::Menu(z)) = k.normal.get(Key::parse("Z").unwrap()) else {
            panic!()
        };
        assert!(z.sticky);
    }

    #[test]
    fn keys_parse_like_helix() {
        assert_eq!(Key::parse("C-A-x").unwrap().to_string(), "C-A-x");
        assert_eq!(Key::parse("S-tab").unwrap().to_string(), "S-tab");
        assert!(Key::parse("").is_none());
        assert!(Key::parse("ab").is_none());
        assert!(Key::parse("C-").is_none());
        assert_eq!(Key::parse("a").unwrap().char(), Some('a'));
        assert_eq!(Key::parse("C-a").unwrap().char(), None);
        assert_eq!(Key::parse("ret").unwrap().char(), None);
    }
}
