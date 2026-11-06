//! The command line while you type into it: `:` commands with Helix's tab completion, `/` and `?`
//! searches, and the regex prompts (`s`, `S`, `K`, `A-K`).

use crossterm::event::{MouseEvent, MouseEventKind};
use glos_helix::{commands, text::Selection};
use ratatui::{buffer::Buffer, layout::Rect};
use unicode_width::UnicodeWidthStr;

use super::{
    component::{Component, Cx, Msg, Outcome},
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
        "delete-comment",
        &[],
        "Delete your latest comment on the thread under the cursor, if it isn't sent",
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

/// What the command line's keys do. `keymap.rs` binds them.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PromptCmd {
    Submit,
    Cancel,
    /// `tab` (`true`) or `S-tab`: the next or previous `:` completion.
    Complete(bool),
    /// `backspace`; on an empty line, it cancels.
    DeleteChar,
}

pub enum PromptKind {
    Command,
    /// A search, forward or back; it previews as you type and `esc` puts the selection back.
    Search {
        forward: bool,
        origin: Selection,
        top: usize,
    },
    /// `/` or `?` in the file tree: the tree's cursor jumps to a match as you type, and `esc`
    /// puts it back.
    TreeSearch {
        forward: bool,
        origin: super::tree::Origin,
    },
    /// `s`, `S`, `K`, `A-K`: previews as you type, as Helix's regex prompts do.
    Regex {
        action: commands::RegexAction,
        origin: Selection,
    },
}

/// The `:` command completions, as pairs of command name and description.
type Completions = Vec<(String, &'static str)>;

pub struct Prompt {
    pub kind: PromptKind,
    pub line: String,
    /// While tabbing through `:` completions: what was typed, and which completion is shown.
    completion: Option<(String, usize)>,
    /// The first completion shown after the wheel scrolls the list. `None` means the list
    /// follows the selected completion. Any key press sets it back to `None`.
    scrolled: Option<usize>,
}

impl Prompt {
    pub fn new(kind: PromptKind) -> Prompt {
        Prompt {
            kind,
            line: String::new(),
            completion: None,
            scrolled: None,
        }
    }

    /// The command line while typing: the prompt's prefix, the line and the cursor.
    fn render_line(&self, buf: &mut Buffer, area: Rect, t: &Theme) {
        let prefix = match self.kind {
            PromptKind::Command => ":",
            PromptKind::Search { .. } | PromptKind::TreeSearch { .. } => "search:",
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
    /// Returns the completions for the typed text, the rectangle to draw them in, and the
    /// index of the first one to show. The list sits at the bottom of `area` and has at most
    /// 10 rows.
    fn completion_list(&self, area: Rect) -> Option<(Completions, Rect, usize)> {
        if !matches!(self.kind, PromptKind::Command) {
            return None;
        }
        let typed = self
            .completion
            .as_ref()
            .map_or(self.line.as_str(), |(t, _)| t.as_str());
        let (_, items) = completions(typed);
        let rows = items.len().min(area.height as usize).min(10);
        if rows == 0 {
            return None;
        }
        // Keep the selected completion in view, unless the wheel has scrolled the list.
        let selected = self.completion.as_ref().map(|(_, i)| *i);
        let first = self
            .scrolled
            .unwrap_or(selected.map_or(0, |i| (i + 1).saturating_sub(rows)))
            .min(items.len() - rows);
        let rect = Rect::new(area.x, area.bottom() - rows as u16, area.width, rows as u16);
        Some((items, rect, first))
    }

    fn render_completion(&self, buf: &mut Buffer, area: Rect, t: &Theme) {
        let Some((items, rect, first)) = self.completion_list(area) else {
            return;
        };
        let rows = rect.height as usize;
        let name_w = items.iter().map(|(n, _)| n.width()).max().unwrap_or(0);
        let selected = self.completion.as_ref().map(|(_, i)| *i);
        let y0 = rect.y;
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

impl Prompt {
    /// Runs one of the command line's commands. What it does to the file (previewing a search,
    /// putting the selection back, running the line) is `App`'s.
    pub fn command(&mut self, cmd: PromptCmd) -> Outcome {
        self.scrolled = None;
        match cmd {
            PromptCmd::Cancel => Outcome::Msg(Msg::CancelPrompt),
            PromptCmd::Submit => Outcome::Msg(Msg::SubmitPrompt),
            PromptCmd::Complete(forward) if matches!(self.kind, PromptKind::Command) => {
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
                let i = match (&self.completion, forward) {
                    (None, false) => n - 1,
                    (None, true) => 0,
                    (Some((_, i)), false) => (i + n - 1) % n,
                    (Some((_, i)), true) => (i + 1) % n,
                };
                self.line = format!("{}{}", &typed[..start], items[i].0);
                self.completion = Some((typed, i));
                Outcome::Consumed
            }
            PromptCmd::Complete(_) => Outcome::Consumed,
            PromptCmd::DeleteChar => {
                self.completion = None;
                match self.line.pop() {
                    Some(_) => Outcome::Msg(Msg::PreviewPrompt),
                    None => Outcome::Msg(Msg::CancelPrompt),
                }
            }
        }
    }

    /// A character typed into the line.
    pub fn text(&mut self, c: char) -> Outcome {
        self.scrolled = None;
        self.completion = None;
        self.line.push(c);
        Outcome::Msg(Msg::PreviewPrompt)
    }
}

impl Component for Prompt {
    /// Clicks are ignored while you type. The wheel scrolls the completion list when the
    /// pointer is over it.
    fn mouse(&mut self, m: MouseEvent, area: Rect, _cx: &Cx) -> Outcome {
        let above = Rect::new(area.x, area.y, area.width, area.height.saturating_sub(2));
        if let Some((items, rect, first)) = self.completion_list(above)
            && rect.contains((m.column, m.row).into())
        {
            let last = items.len() - rect.height as usize;
            match m.kind {
                MouseEventKind::ScrollDown => self.scrolled = Some((first + 3).min(last)),
                MouseEventKind::ScrollUp => self.scrolled = Some(first.saturating_sub(3)),
                _ => {}
            }
        }
        Outcome::Consumed
    }

    /// The line on the screen's (`area`'s) last row, and `:` completions above the statusline.
    fn render(&mut self, buf: &mut Buffer, area: Rect, cx: &Cx) {
        if area.height < 3 {
            return;
        }
        let bottom = area.bottom() - 1;
        self.render_line(buf, Rect::new(area.x, bottom, area.width, 1), cx.theme);
        let above = Rect::new(area.x, area.y, area.width, area.height - 2);
        self.render_completion(buf, above, cx.theme);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Presses `key` on the command line as `App` does: a bound command, or a plain character
    /// typed. Anything else does nothing.
    fn press(p: &mut Prompt, key: &str) -> Outcome {
        use crate::ui::keymap::{Action, Context, Key, Keymaps, Node};
        let key = Key::parse(key).unwrap();
        match Keymaps::default().get(Context::Prompt).get(key) {
            Some(Node::Cmd(c)) => match c.action {
                Action::Prompt(cmd) => p.command(cmd),
                _ => panic!("{} isn't a command line command", c.name),
            },
            _ => key.char().map_or(Outcome::Consumed, |c| p.text(c)),
        }
    }

    #[test]
    fn keys_edit_the_line_and_ask_app_for_the_rest() {
        let mut p = Prompt::new(PromptKind::Command);
        assert_eq!(press(&mut p, "q"), Outcome::Msg(Msg::PreviewPrompt));
        assert_eq!(p.line, "q");
        // Ctrl and alt characters aren't typed.
        assert_eq!(press(&mut p, "A-x"), Outcome::Consumed);
        assert_eq!(p.line, "q");
        // Tab walks the completions; shift-tab walks back; typing starts again from the text.
        assert_eq!(press(&mut p, "tab"), Outcome::Consumed);
        assert_eq!(p.line, "quit");
        press(&mut p, "tab");
        assert_eq!(p.line, "quit!");
        press(&mut p, "S-tab");
        assert_eq!(p.line, "quit");
        assert_eq!(press(&mut p, "ret"), Outcome::Msg(Msg::SubmitPrompt));
        for _ in 0..4 {
            assert_eq!(press(&mut p, "backspace"), Outcome::Msg(Msg::PreviewPrompt));
        }
        assert_eq!(
            press(&mut p, "backspace"),
            Outcome::Msg(Msg::CancelPrompt),
            "backspace on an empty line cancels"
        );
        assert_eq!(press(&mut p, "esc"), Outcome::Msg(Msg::CancelPrompt));
        assert_eq!(press(&mut p, "C-c"), Outcome::Msg(Msg::CancelPrompt));
        // A search has no completions: tab does nothing.
        let mut s = Prompt::new(PromptKind::Search {
            forward: true,
            origin: Selection::point(0),
            top: 0,
        });
        assert_eq!(press(&mut s, "tab"), Outcome::Consumed);
        assert_eq!(s.line, "");
    }

    #[test]
    fn draws_the_line_at_the_bottom_and_completions_above_the_statusline() {
        let theme =
            crate::theme::Theme::load("default", crate::theme::ColorSupport::TrueColor).unwrap();
        let area = Rect::new(0, 0, 40, 6);
        let cx = Cx {
            screen: area,
            theme: &theme,
            state: None,
        };
        let mut p = Prompt::new(PromptKind::Command);
        p.line = "sub".into();
        let mut buf = Buffer::empty(area);
        p.render(&mut buf, area, &cx);
        let row = |y: u16| -> String {
            (0..area.width)
                .map(|x| buf[(x, y)].symbol())
                .collect::<String>()
                .trim_end()
                .to_string()
        };
        assert_eq!(row(5), ":sub");
        assert_eq!(row(4), "", "the statusline's row is left alone");
        assert!(row(3).trim_start().starts_with("submit"));
    }
}
