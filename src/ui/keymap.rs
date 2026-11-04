//! Helix's keymap: a trie of keys per mode, with menus (`g`, `m`, `[`, …) that open the key
//! help panel. Built from Helix's default keymap at 25.07.1, keeping only what applies to glos.

use std::fmt;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::commands::{self as c, Ctx};

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
    suspend, "Suspend and return to shell", App(AppAction::Suspend);
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
        let mut mods = e.modifiers & (KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SHIFT);
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
}

impl Menu {
    pub fn get(&self, key: Key) -> Option<&Node> {
        self.keys.iter().find(|(k, _)| *k == key).map(|(_, n)| n)
    }

    /// The key help panel's rows: keys bound to the same thing are joined (`h, left`).
    pub fn help(&self) -> Vec<(String, &'static str)> {
        let mut rows: Vec<(String, &'static str, *const ())> = vec![];
        for (key, node) in &self.keys {
            let (doc, id) = match node {
                Node::Cmd(c) => (c.doc, *c as *const Command as *const ()),
                Node::Menu(m) => (m.name, m as *const Menu as *const ()),
            };
            match rows.iter_mut().find(|r| r.2 == id) {
                Some(r) => r.0 = format!("{}, {key}", r.0),
                None => rows.push((key.to_string(), doc, id)),
            }
        }
        rows.into_iter().map(|(k, d, _)| (k, d)).collect()
    }
}

enum Def {
    Cmd(&'static str, &'static str),
    Menu(&'static str, &'static str, Vec<Def>),
}

fn cmd(keys: &'static str, name: &'static str) -> Def {
    Def::Cmd(keys, name)
}

fn menu(key: &'static str, name: &'static str, defs: Vec<Def>) -> Def {
    Def::Menu(key, name, defs)
}

fn build(name: &'static str, defs: Vec<Def>) -> Menu {
    let mut keys = vec![];
    for def in defs {
        match def {
            Def::Cmd(ks, c) => {
                let command = command(c).unwrap_or_else(|| panic!("no command {c}"));
                for k in ks.split('|') {
                    keys.push((Key::parse(k).unwrap_or_else(|| panic!("bad key {k}")), Node::Cmd(command)));
                }
            }
            Def::Menu(k, name, defs) => {
                keys.push((Key::parse(k).unwrap(), Node::Menu(build(name, defs))));
            }
        }
    }
    Menu { name, keys }
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
        menu("g", "Goto", vec![
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
        ]),
        cmd(":", "command_mode"),
        cmd(";", "collapse_selection"),
        cmd("A-;", "flip_selections"),
        cmd("%", "select_all"),
        cmd("x", "extend_line_below"),
        cmd("X", "extend_to_line_bounds"),
        menu("m", "Match", vec![
            cmd("m", "match_brackets"),
            cmd("a", "select_textobject_around"),
            cmd("i", "select_textobject_inner"),
        ]),
        menu("[", "Left bracket", vec![
            cmd("g", "goto_prev_change"),
            cmd("G", "goto_first_change"),
        ]),
        menu("]", "Right bracket", vec![
            cmd("g", "goto_next_change"),
            cmd("G", "goto_last_change"),
        ]),
        cmd(",", "keep_primary_selection"),
        cmd("C-b|pageup", "page_up"),
        cmd("C-f|pagedown", "page_down"),
        cmd("C-u", "page_cursor_half_up"),
        cmd("C-d", "page_cursor_half_down"),
        cmd("C-z", "suspend"),
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
        select.retain(|d| !matches!(d, Def::Cmd("v", _)));
        select.push(cmd("v", "normal_mode"));
        select.push(cmd("esc", "exit_select_mode"));
        Keymaps { normal: build("Normal mode", normal_defs()), select: build("Select mode", select) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_round_trip() {
        for k in ["h", "G", "C-d", "A-;", "space", "ret", "esc", "pageup", "%", "minus"] {
            assert_eq!(Key::parse(k).unwrap().to_string(), k);
        }
        let shifted = Key::from(KeyEvent::new(KeyCode::Char('G'), KeyModifiers::SHIFT));
        assert_eq!(shifted, Key::parse("G").unwrap());
    }

    #[test]
    fn help_joins_aliases() {
        let k = Keymaps::default();
        let help = k.normal.help();
        assert!(help.contains(&("h, left".to_string(), "Move left")));
        let Some(Node::Menu(g)) = k.normal.get(Key::parse("g").unwrap()) else { panic!() };
        assert_eq!(g.name, "Goto");
        assert!(matches!(k.select.get(Key::parse("v").unwrap()), Some(Node::Cmd(c)) if c.name == "normal_mode"));
    }
}
