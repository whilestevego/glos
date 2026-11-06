//! What the parts of the screen share, as Helix's compositor layers do: a picker, the command
//! line, the file tree and the comment box each handle their own keys and clicks and draw
//! themselves. They read the review through [`Cx`] and ask `App` for any change beyond their own
//! state with a [`Msg`].

use crossterm::event::MouseEvent;
use ratatui::{buffer::Buffer, layout::Rect};

use super::{State, picker::Target};
use crate::theme::Theme;

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
    /// From the file tree: `/` (`true`) or `?`, to search its rows.
    TreeSearch(bool),
    /// From the file tree: `n` (`true`) or `N`, to repeat the last search.
    TreeSearchNext(bool),
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
pub struct Cx<'a> {
    /// The whole screen, as last drawn.
    pub screen: Rect,
    pub theme: &'a Theme,
    /// The open review; `None` in `glos dashboard`, before one is open.
    pub state: Option<&'a State>,
}

pub trait Component {
    /// A click or the wheel; `area` is where the component was drawn.
    fn mouse(&mut self, _m: MouseEvent, _area: Rect, _cx: &Cx) -> Outcome {
        Outcome::Ignored
    }

    /// Draws it in `area`: the screen for a picker or the command line, else its own place.
    fn render(&mut self, buf: &mut Buffer, area: Rect, cx: &Cx);
}
