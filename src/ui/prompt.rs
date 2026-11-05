//! The command line while you type into it: `:` commands with Helix's tab completion, `/` and `?`
//! searches, and the regex prompts (`s`, `S`, `K`, `A-K`).

use crossterm::event::{KeyCode, KeyModifiers, MouseEvent};
use glos_helix::{commands, text::Selection};
use ratatui::{buffer::Buffer, layout::Rect};
use unicode_width::UnicodeWidthStr;

use super::{
    component::{Component, Cx, Msg, Outcome},
    keymap::Key,
    view::clean,
};
use crate::theme::Theme;

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
pub fn completions(line: &str) -> (usize, Vec<(String, &'static str)>) {
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

pub enum PromptKind {
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

pub struct Prompt {
    pub kind: PromptKind,
    pub line: String,
    /// While tabbing through `:` completions: what was typed, and which completion is shown.
    completion: Option<(String, usize)>,
}

impl Prompt {
    pub fn new(kind: PromptKind) -> Prompt {
        Prompt {
            kind,
            line: String::new(),
            completion: None,
        }
    }

    /// The command line while typing: the prompt's prefix, the line and the cursor.
    pub fn render_line(&self, buf: &mut Buffer, area: Rect, t: &Theme) {
        let prefix = match self.kind {
            PromptKind::Command => ":",
            PromptKind::Search { .. } => "search:",
            PromptKind::Regex { action, .. } => action.prompt(),
        };
        let shown = format!("{prefix}{}", clean(&self.line));
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
    }

    /// `:` completions above the command line, as Helix lists them, with what each does.
    pub fn render_completion(&self, buf: &mut Buffer, area: Rect, t: &Theme) {
        let Prompt {
            kind: PromptKind::Command,
            line,
            completion,
        } = self
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
}

impl Component for Prompt {
    /// Every key is the prompt's while it's open. What it does to the file (previewing a search,
    /// putting the selection back, running the line) is `App`'s.
    fn key(&mut self, key: Key, _cx: &Cx) -> Outcome {
        let ctrl = key.mods.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Esc => Outcome::Msg(Msg::CancelPrompt),
            KeyCode::Char('c') if ctrl => Outcome::Msg(Msg::CancelPrompt),
            KeyCode::Enter => Outcome::Msg(Msg::SubmitPrompt),
            KeyCode::Tab | KeyCode::BackTab if matches!(self.kind, PromptKind::Command) => {
                // Helix's completion: each tab shows the next match for what was typed.
                let typed = match &self.completion {
                    Some((t, _)) => t.clone(),
                    None => self.line.clone(),
                };
                let (start, items) = completions(&typed);
                if items.is_empty() {
                    return Outcome::Consumed;
                }
                let n = items.len();
                let back = key.code == KeyCode::BackTab || key.mods.contains(KeyModifiers::SHIFT);
                let i = match (&self.completion, back) {
                    (None, true) => n - 1,
                    (None, false) => 0,
                    (Some((_, i)), true) => (i + n - 1) % n,
                    (Some((_, i)), false) => (i + 1) % n,
                };
                self.line = format!("{}{}", &typed[..start], items[i].0);
                self.completion = Some((typed, i));
                Outcome::Consumed
            }
            KeyCode::Backspace => {
                self.completion = None;
                match self.line.pop() {
                    Some(_) => Outcome::Msg(Msg::PreviewPrompt),
                    None => Outcome::Msg(Msg::CancelPrompt),
                }
            }
            KeyCode::Char(c)
                if !key
                    .mods
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.completion = None;
                self.line.push(c);
                Outcome::Msg(Msg::PreviewPrompt)
            }
            _ => Outcome::Consumed,
        }
    }

    /// Clicks do nothing while typing.
    fn mouse(&mut self, _m: MouseEvent, _area: Rect, _cx: &Cx) -> Outcome {
        Outcome::Consumed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode, mods: KeyModifiers) -> Key {
        Key { code, mods }
    }

    #[test]
    fn keys_edit_the_line_and_ask_app_for_the_rest() {
        let cx = Cx {
            screen: Rect::new(0, 0, 80, 24),
        };
        let none = KeyModifiers::NONE;
        let mut p = Prompt::new(PromptKind::Command);
        assert_eq!(
            p.key(key(KeyCode::Char('q'), none), &cx),
            Outcome::Msg(Msg::PreviewPrompt)
        );
        assert_eq!(p.line, "q");
        // Ctrl and alt characters aren't typed, and every key stays the prompt's.
        assert_eq!(
            p.key(key(KeyCode::Char('x'), KeyModifiers::ALT), &cx),
            Outcome::Consumed
        );
        assert_eq!(p.key(key(KeyCode::F(1), none), &cx), Outcome::Consumed);
        assert_eq!(p.line, "q");
        // Tab walks the completions; shift-tab walks back; typing starts again from the text.
        assert_eq!(p.key(key(KeyCode::Tab, none), &cx), Outcome::Consumed);
        assert_eq!(p.line, "quit");
        p.key(key(KeyCode::Tab, none), &cx);
        assert_eq!(p.line, "quit!");
        p.key(key(KeyCode::BackTab, none), &cx);
        assert_eq!(p.line, "quit");
        assert_eq!(
            p.key(key(KeyCode::Enter, none), &cx),
            Outcome::Msg(Msg::SubmitPrompt)
        );
        for _ in 0..4 {
            assert_eq!(
                p.key(key(KeyCode::Backspace, none), &cx),
                Outcome::Msg(Msg::PreviewPrompt)
            );
        }
        assert_eq!(
            p.key(key(KeyCode::Backspace, none), &cx),
            Outcome::Msg(Msg::CancelPrompt),
            "backspace on an empty line cancels"
        );
        assert_eq!(
            p.key(key(KeyCode::Esc, none), &cx),
            Outcome::Msg(Msg::CancelPrompt)
        );
        assert_eq!(
            p.key(key(KeyCode::Char('c'), KeyModifiers::CONTROL), &cx),
            Outcome::Msg(Msg::CancelPrompt)
        );
        // A search has no completions: tab does nothing.
        let mut s = Prompt::new(PromptKind::Search {
            forward: true,
            origin: Selection::point(0),
            top: 0,
        });
        assert_eq!(s.key(key(KeyCode::Tab, none), &cx), Outcome::Consumed);
        assert_eq!(s.line, "");
    }
}
