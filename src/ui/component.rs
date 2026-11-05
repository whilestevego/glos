//! What the parts of the screen that take keys share: a picker, the command line, the file tree
//! and the comment box each handle their own keys and clicks, and ask `App` for anything beyond
//! their own state with a [`Msg`].
//!
//! Drawing isn't part of the trait: each part needs different things from `App` to draw (the
//! picker's preview, the box's selection colours), so each has its own `render`.

use crossterm::event::MouseEvent;
use ratatui::layout::Rect;

use super::{keymap::Key, picker::Target};

/// What a component did with a key or click.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Not one of its keys: the next layer gets it.
    Ignored,
    /// Handled, and nothing for `App` to do.
    Consumed,
    /// Handled, and `App` has to act on it.
    Msg(Msg),
}

/// What a component asks `App` to do.
#[derive(Debug, PartialEq, Eq)]
pub enum Msg {
    /// From a picker: close it.
    Close,
    /// From a picker or the file tree: open a file or thread, run a command, or go back (`Target::Nothing`).
    Open(Target),
    /// From the command line: the line changed, so a search or regex shows its matches.
    PreviewPrompt,
    /// From the command line: `esc`, or backspace on an empty line.
    CancelPrompt,
    /// From the command line: `ret`.
    SubmitPrompt,
    /// From the file tree: `q`.
    CloseTree,
    /// From the comment box: its text changed, so it's parsed and its draft saved.
    BoxEdited,
    /// From the comment box: its cursor moved, or it left insert mode.
    BoxMoved,
    /// From the comment box: `i` or `a`, into insert mode.
    BoxInsert,
    /// From the comment box: `ret`.
    SaveBox,
    /// From the comment box: `q`.
    CloseBox,
}

/// What every component may read.
pub struct Cx {
    /// The whole screen, as last drawn.
    pub screen: Rect,
}

pub trait Component {
    fn key(&mut self, key: Key, cx: &Cx) -> Outcome;

    /// A click or the wheel; `area` is where the component was drawn.
    fn mouse(&mut self, _m: MouseEvent, _area: Rect, _cx: &Cx) -> Outcome {
        Outcome::Ignored
    }
}
