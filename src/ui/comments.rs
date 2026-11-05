//! Review actions in the UI: the comment box, threads drawn under their lines, and `:editor`.

use std::collections::{BTreeMap, HashMap, HashSet};

use anyhow::{Context, Result, bail};
use crossterm::event::KeyCode;
use glos_helix::{
    layout::LineAnnotation,
    syntax::Loader,
    text::{
        Position, Range, Rope, Selection, Tendril, Transaction, next_grapheme_boundary,
        prev_grapheme_boundary,
    },
};
use ratatui::style::Style;

use super::view::File;
use crate::{
    review::{Anchor, Thread, YOU},
    theme::Theme,
};

/// What a comment box writes when it's saved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Place {
    /// A new thread on lines of a file.
    Lines(Anchor),
    /// A new review-level thread.
    Review,
    /// A reply in a thread, by its id.
    Reply(String),
    /// A change to one of your comments, by its id.
    Edit(String),
}

impl Place {
    /// Where its draft is kept in the store.
    pub fn key(&self) -> String {
        match self {
            Place::Lines(a) => format!("line:{}:{}-{}", a.path, a.start, a.end),
            Place::Review => "review".into(),
            Place::Reply(t) => format!("reply:{t}"),
            Place::Edit(c) => format!("edit:{c}"),
        }
    }

    pub fn title(&self) -> String {
        match self {
            Place::Lines(a) => format!("Comment on {a}"),
            Place::Review => "Comment on the review".into(),
            Place::Reply(_) => "Reply".into(),
            Place::Edit(_) => "Edit comment".into(),
        }
    }
}

/// The comment box: a small Helix-style buffer, opened in insert mode. In normal mode the
/// usual motions and selections work on it, `ret` saves and `q` closes.
pub struct CommentBox {
    pub place: Place,
    pub file: File,
    pub insert: bool,
    /// An empty save of an edit deletes the comment, after asking once.
    pub confirm_delete: bool,
}

impl CommentBox {
    pub fn new(place: Place, text: &str, loader: &Loader) -> CommentBox {
        let mut file = File::scratch(text, loader);
        // In insert mode the cursor is a point; it starts after any draft.
        file.sel = Selection::point(file.text.len_chars());
        CommentBox {
            place,
            file,
            insert: true,
            confirm_delete: false,
        }
    }

    /// The text as it would be saved.
    pub fn body(&self) -> String {
        self.file.text.to_string().trim().to_string()
    }

    pub fn set_text(&mut self, text: &str, loader: &Loader) {
        self.file.text = Rope::from(text);
        self.file.sel = Selection::point(self.file.text.len_chars());
        self.file.reparse(loader);
    }

    /// `i` and `a`: insert before or after each selection.
    pub fn enter_insert(&mut self, after: bool) {
        let sel = self.file.sel.clone();
        self.file.sel = sel.transform(|r| Range::point(if after { r.to() } else { r.from() }));
        self.insert = true;
    }

    /// `esc`: back to normal mode, the cursor on the character before it, as in Helix.
    pub fn leave_insert(&mut self) {
        let text = self.file.text.slice(..);
        let sel = self.file.sel.clone().transform(|r| {
            let p = prev_grapheme_boundary(text, r.head);
            Range::new(p, next_grapheme_boundary(text, p))
        });
        self.file.sel = sel.ensure_invariants(text);
        self.insert = false;
    }

    /// A key in insert mode. Returns whether the text changed.
    pub fn insert_key(&mut self, code: KeyCode, loader: &Loader) -> bool {
        let text = self.file.text.slice(..);
        let tx = match code {
            KeyCode::Char(c) => Transaction::insert(
                &self.file.text,
                &self.file.sel,
                Tendril::from(c.to_string()),
            ),
            KeyCode::Enter => {
                Transaction::insert(&self.file.text, &self.file.sel, Tendril::from("\n"))
            }
            KeyCode::Tab => {
                Transaction::insert(&self.file.text, &self.file.sel, Tendril::from("\t"))
            }
            KeyCode::Backspace => {
                Transaction::change_by_selection(&self.file.text, &self.file.sel, |r| {
                    (prev_grapheme_boundary(text, r.head), r.head, None)
                })
            }
            KeyCode::Delete => {
                Transaction::change_by_selection(&self.file.text, &self.file.sel, |r| {
                    (r.head, next_grapheme_boundary(text, r.head), None)
                })
            }
            KeyCode::Left
            | KeyCode::Right
            | KeyCode::Up
            | KeyCode::Down
            | KeyCode::Home
            | KeyCode::End => {
                let sel = self.file.sel.clone();
                self.file.sel = sel.transform(|r| Range::point(move_point(text, r.head, code)));
                return false;
            }
            _ => return false,
        };
        tx.apply(&mut self.file.text);
        self.file.sel = self.file.sel.clone().map(tx.changes());
        self.file.reparse(loader);
        true
    }
}

/// Moves an insert-mode cursor with the arrow keys, keeping the column across lines.
fn move_point(text: glos_helix::text::RopeSlice, pos: usize, code: KeyCode) -> usize {
    let line = text.char_to_line(pos);
    let start = text.line_to_char(line);
    let line_end = |l: usize| glos_helix::text::line_end_char_index(&text, l);
    let to_line =
        |l: usize| text.line_to_char(l) + (pos - start).min(line_end(l) - text.line_to_char(l));
    match code {
        KeyCode::Left => prev_grapheme_boundary(text, pos),
        KeyCode::Right => next_grapheme_boundary(text, pos),
        KeyCode::Up if line > 0 => to_line(line - 1),
        KeyCode::Down if line + 1 < text.len_lines() => to_line(line + 1),
        KeyCode::Home => start,
        KeyCode::End => line_end(line),
        _ => pos,
    }
}

/// One row drawn under a line for a thread.
pub struct ThreadRow {
    pub text: String,
    pub style: Style,
    /// Which thread, as an index into the threads it was built from.
    pub thread: usize,
}

/// The threads of one file as drawn: rows under their last line, and gutter marks.
#[derive(Default)]
pub struct Threads {
    /// Rows under a line (0-based).
    pub rows: BTreeMap<usize, Vec<ThreadRow>>,
    /// Lines (0-based, inclusive) with a thread, marked in the diagnostics gutter.
    pub marks: Vec<(usize, usize, Style)>,
}

/// Lays out the threads of `path` from `threads`: each collapsed to one line unless it's in
/// `expanded`, like Helix's inline diagnostics. Your threads take `info`'s colour and agents'
/// `hint`'s, but every comment also names its author.
/// `here` is the commit on the new side of what's shown (`None` for your files): threads made
/// on another version say which.
pub fn layout(
    threads: &[Thread],
    path: &str,
    expanded: &HashSet<String>,
    theme: &Theme,
    here: Option<&str>,
    width: usize,
) -> Threads {
    let mut out = Threads::default();
    for (i, t) in threads.iter().enumerate() {
        let Some(a) = t.anchor.as_ref().filter(|a| a.path == path) else {
            continue;
        };
        let started_by_you = t.comments.first().is_none_or(|c| c.author == YOU);
        let style = theme.get(if started_by_you { "info" } else { "hint" });
        let (start, end) = (a.start as usize - 1, a.end as usize - 1);
        out.marks.push((start, end, style));
        let rows = out.rows.entry(end).or_default();
        // Each row wraps to the width, its continuations indented as it is.
        let mut wrapped = |text: String, style: Style, max: usize| {
            for text in wrap(&text, width, max) {
                rows.push(ThreadRow {
                    text,
                    style,
                    thread: i,
                })
            }
        };
        let mut row = |text: String, style: Style| wrapped(text, style, usize::MAX);
        let unsent = t.comments.iter().filter(|c| c.unsent()).count();
        let mut tags = vec![];
        if t.unread() {
            tags.push("unread".to_string());
        }
        if t.comments.len() > 1 {
            tags.push(format!("{} comments", t.comments.len()));
        }
        if unsent > 0 {
            tags.push("unsent".to_string());
        }
        if t.status != "open" {
            tags.push(t.status.clone());
        }
        if t.on.new_rev.as_deref() != here {
            tags.push(match &t.on.new_rev {
                Some(c) => format!("on {}", &c[..c.len().min(7)]),
                None => "on your files".into(),
            });
        }
        let tags: String = tags.iter().map(|s| format!(" · {s}")).collect();
        if !expanded.contains(&t.id) {
            let first = t.comments.first();
            let line = first.map_or("", |c| c.body.lines().next().unwrap_or(""));
            let author = first.map_or(String::new(), who);
            // Closed, a thread stays short: two rows at most, `…` if there's more.
            wrapped(format!("▸ {author}: {line}{tags}"), style, 2);
            continue;
        }
        row(format!("▾ thread {}{tags}", t.id), style);
        // An outdated thread's code is gone: show it as it was.
        if t.status == "outdated"
            && let Some(text) = &t.anchor_text
        {
            row("  the code it was on:".into(), theme.get("ui.virtual"));
            for l in text.lines().take(5) {
                row(format!("  │ {l}"), theme.get("ui.virtual"));
            }
        }
        for c in &t.comments {
            let mut head = format!("  {}", who(c));
            if c.unsent() {
                head += " · unsent";
            }
            if c.edited_at.is_some() {
                head += " · edited";
            }
            row(head, style);
            for l in c.body.lines() {
                row(format!("    {l}"), theme.get("ui.text"));
            }
        }
    }
    out
}

/// `text` in rows of at most `width` columns, broken between words where it can be, each
/// continuation indented like the first row (past a leading `▸ ` or `▾ `). At most `max` rows;
/// the last ends in `…` if text was left out. A width of 0 (not drawn yet) doesn't wrap.
fn wrap(text: &str, width: usize, max: usize) -> Vec<String> {
    use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};
    if width == 0 || text.width() <= width {
        return vec![text.to_string()];
    }
    let (lead, rest) = text.split_at(text.len() - text.trim_start_matches([' ', '▸', '▾']).len());
    let indent = " ".repeat(lead.width().min(width / 2));
    let mut rows = vec![];
    let mut cur = lead.to_string();
    // Whether `cur` has any text yet, past its lead or indent.
    let mut started = false;
    let mut cut = false;
    'words: for word in rest.split(' ') {
        let gap = usize::from(started);
        if cur.width() + gap + word.width() <= width {
            if started {
                cur.push(' ');
            }
            cur.push_str(word);
            started = true;
            continue;
        }
        if started {
            if rows.len() + 1 == max {
                cut = true;
                break;
            }
            rows.push(std::mem::replace(&mut cur, indent.clone()));
            started = false;
            if cur.width() + word.width() <= width {
                cur.push_str(word);
                started = true;
                continue;
            }
        }
        // A word wider than a row is broken where it reaches the edge.
        for ch in word.chars() {
            if cur.width() + ch.width().unwrap_or(0) > width {
                if rows.len() + 1 == max {
                    cut = true;
                    break 'words;
                }
                rows.push(std::mem::replace(&mut cur, indent.clone()));
            }
            cur.push(ch);
            started = true;
        }
    }
    if cut {
        while cur.width() + 1 > width {
            cur.pop();
        }
        cur.truncate(cur.trim_end().len());
        cur.push('…');
    }
    rows.push(cur);
    rows
}

/// A comment's author, with agents' notes made before review marked as such.
fn who(c: &crate::review::Comment) -> String {
    if c.kind == "annotation" {
        format!("{} (note)", c.author)
    } else {
        c.author.clone()
    }
}

/// Thread rows as helix-core line annotations, so scrolling and the mouse count them as the
/// screen shows them. Keyed by the char index just after a line's newline: the formatter also
/// asks at soft-wrap breaks, which must not get the rows again.
pub struct ThreadLines(pub HashMap<usize, usize>);

impl ThreadLines {
    pub fn new(threads: &Threads, text: glos_helix::text::RopeSlice) -> ThreadLines {
        ThreadLines(
            threads
                .rows
                .iter()
                .filter(|(line, _)| **line + 1 < text.len_lines())
                .map(|(line, rows)| (text.line_to_char(line + 1), rows.len()))
                .collect(),
        )
    }
}

impl LineAnnotation for ThreadLines {
    fn insert_virtual_lines(&mut self, char_idx: usize, _: Position, _: usize) -> Position {
        Position::new(self.0.get(&char_idx).copied().unwrap_or(0), 0)
    }
}

/// `:editor`: the text in `$VISUAL` or `$EDITOR` (run through the shell, so it may have
/// arguments), and what was saved. The trailing newline editors add is dropped.
pub fn external_edit(editor: &str, text: &str) -> Result<String> {
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("glos-comment-{}-{n}.md", std::process::id()));
    std::fs::write(&path, text).with_context(|| format!("writing {}", path.display()))?;
    let status = std::process::Command::new("sh")
        .arg("-c")
        .arg(format!("{editor} \"$1\""))
        .arg("sh")
        .arg(&path)
        .status()
        .with_context(|| format!("running {editor}"));
    let result = std::fs::read_to_string(&path);
    let _ = std::fs::remove_file(&path);
    if !status?.success() {
        bail!("{editor} failed; the comment is unchanged");
    }
    Ok(result?.trim_end_matches('\n').to_string())
}

pub fn editor() -> String {
    std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .unwrap_or_else(|_| "vi".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::ColorSupport;
    use glos_helix::test::plain;

    fn loader() -> Loader {
        Loader::new(
            Theme::load("default", ColorSupport::TrueColor)
                .unwrap()
                .scopes(),
        )
        .unwrap()
    }

    fn typed(b: &mut CommentBox, s: &str, loader: &Loader) {
        for c in s.chars() {
            let code = if c == '\n' {
                KeyCode::Enter
            } else {
                KeyCode::Char(c)
            };
            b.insert_key(code, loader);
        }
    }

    #[test]
    fn typing_editing_and_modes() {
        let l = loader();
        let mut b = CommentBox::new(Place::Review, "", &l);
        typed(&mut b, "helo", &l);
        assert_eq!(plain(b.file.text.clone(), &b.file.sel), "helo#[|]#");
        b.insert_key(KeyCode::Left, &l);
        typed(&mut b, "l", &l);
        assert_eq!(b.body(), "hello");
        b.insert_key(KeyCode::Backspace, &l);
        b.insert_key(KeyCode::Backspace, &l);
        assert_eq!(b.body(), "heo");
        b.insert_key(KeyCode::Delete, &l);
        assert_eq!(b.body(), "he");
        // esc puts a one-character cursor on the character before, as Helix does.
        b.leave_insert();
        assert!(!b.insert);
        assert_eq!(plain(b.file.text.clone(), &b.file.sel), "h#[e|]#");
        b.enter_insert(true);
        typed(&mut b, "llo", &l);
        assert_eq!(b.body(), "hello");
        b.leave_insert();
        b.enter_insert(false);
        typed(&mut b, "!", &l);
        assert_eq!(b.body(), "hell!o");
        // Backspace at the start does nothing.
        let mut b = CommentBox::new(Place::Review, "", &l);
        b.insert_key(KeyCode::Backspace, &l);
        b.leave_insert();
        assert_eq!(b.body(), "");
    }

    #[test]
    fn arrows_keep_the_column() {
        let l = loader();
        let mut b = CommentBox::new(Place::Review, "abcd\nxy\nlonger", &l);
        assert_eq!(b.file.sel.primary().head, 14);
        b.insert_key(KeyCode::Up, &l);
        assert_eq!(b.file.sel.primary().head, 7, "clamped to the end of `xy`");
        b.insert_key(KeyCode::Up, &l);
        assert_eq!(b.file.sel.primary().head, 2);
        b.insert_key(KeyCode::Up, &l);
        assert_eq!(b.file.sel.primary().head, 2, "no line above");
        b.insert_key(KeyCode::End, &l);
        assert_eq!(b.file.sel.primary().head, 4);
        b.insert_key(KeyCode::Home, &l);
        assert_eq!(b.file.sel.primary().head, 0);
        typed(&mut b, "> ", &l);
        assert_eq!(b.body(), "> abcd\nxy\nlonger");
    }

    #[test]
    fn places_have_their_own_drafts() {
        let a = Anchor {
            path: "src/a.rs".into(),
            start: 3,
            end: 5,
        };
        assert_eq!(Place::Lines(a.clone()).key(), "line:src/a.rs:3-5");
        assert_eq!(Place::Lines(a).title(), "Comment on src/a.rs:3-5");
        assert_eq!(Place::Reply("t".into()).key(), "reply:t");
        assert_eq!(Place::Edit("c".into()).key(), "edit:c");
    }

    fn thread(id: &str, path: &str, start: u32, end: u32, bodies: &[(&str, &str)]) -> Thread {
        Thread {
            id: id.into(),
            anchor: Some(Anchor {
                path: path.into(),
                start,
                end,
            }),
            status: "open".into(),
            created_round: 1,
            anchor_text: None,
            last_seen_at: None,
            on: Default::default(),
            comments: bodies
                .iter()
                .map(|(author, body)| crate::review::Comment {
                    id: format!("c-{body}"),
                    author: author.to_string(),
                    kind: "comment".into(),
                    body: body.to_string(),
                    created_at: String::new(),
                    edited_at: None,
                    submitted_in: (*author != YOU).then_some(1),
                })
                .collect(),
        }
    }

    #[test]
    fn threads_collapse_to_one_line_and_open() {
        let theme = Theme::load("default", ColorSupport::TrueColor).unwrap();
        let threads = [
            thread(
                "t1",
                "a.rs",
                2,
                3,
                &[(YOU, "why this?\nmore"), ("claude", "because")],
            ),
            thread("t2", "b.rs", 1, 1, &[(YOU, "elsewhere")]),
            thread("t3", "a.rs", 3, 3, &[("claude", "a note")]),
        ];
        let rows = |expanded: &HashSet<String>| -> Vec<(usize, String)> {
            layout(&threads, "a.rs", expanded, &theme, None, 80)
                .rows
                .into_iter()
                .flat_map(|(l, rs)| rs.into_iter().map(move |r| (l, r.text)))
                .collect()
        };
        assert_eq!(
            rows(&HashSet::new()),
            [
                (
                    2,
                    "▸ you: why this? · unread · 2 comments · unsent".to_string()
                ),
                (2, "▸ claude: a note · unread".to_string())
            ]
        );
        let open = HashSet::from(["t1".to_string()]);
        let texts: Vec<String> = rows(&open).into_iter().map(|r| r.1).collect();
        assert_eq!(
            texts,
            [
                "▾ thread t1 · unread · 2 comments · unsent",
                "  you · unsent",
                "    why this?",
                "    more",
                "  claude",
                "    because",
                "▸ claude: a note · unread"
            ]
        );
        let l = layout(&threads, "a.rs", &HashSet::new(), &theme, None, 80);
        assert_eq!(l.marks.len(), 2);
        assert_eq!((l.marks[0].0, l.marks[0].1), (1, 2));
        // Agents' threads are drawn in another colour.
        assert_ne!(l.marks[0].2, l.marks[1].2);
    }

    #[test]
    fn long_rows_wrap_between_words_and_keep_their_indent() {
        assert_eq!(wrap("short", 20, usize::MAX), ["short"]);
        assert_eq!(
            wrap("not wrapped before drawing", 0, 1),
            ["not wrapped before drawing"]
        );
        assert_eq!(
            wrap("    one two three four five", 14, usize::MAX),
            ["    one two", "    three four", "    five"]
        );
        // A word longer than a row is broken at the edge.
        assert_eq!(
            wrap("    abcdefghijkl", 10, usize::MAX),
            ["    abcdef", "    ghijkl"]
        );
        // The marker's width counts in the indent; wide characters take two columns.
        assert_eq!(
            wrap("▸ you: 漢字 漢字 漢字", 12, usize::MAX),
            ["▸ you: 漢字", "  漢字 漢字"]
        );
        // At most `max` rows, the last ending in `…`.
        assert_eq!(
            wrap("▸ you: one two three four five six", 14, 2),
            ["▸ you: one two", "  three four…"]
        );
        for row in wrap(&"word ".repeat(40), 17, usize::MAX) {
            assert!(
                unicode_width::UnicodeWidthStr::width(row.as_str()) <= 17,
                "{row:?}"
            );
        }
    }

    #[test]
    fn open_threads_wrap_in_full_and_closed_ones_stay_short() {
        let theme = Theme::load("default", ColorSupport::TrueColor).unwrap();
        let body = "this comment is long enough that it has to wrap onto several rows";
        let threads = [thread("t1", "a.rs", 1, 1, &[(YOU, body)])];
        let texts = |expanded: &HashSet<String>| -> Vec<String> {
            layout(&threads, "a.rs", expanded, &theme, None, 30).rows[&0]
                .iter()
                .map(|r| r.text.clone())
                .collect()
        };
        let closed = texts(&HashSet::new());
        assert_eq!(closed.len(), 2, "{closed:?}");
        assert!(closed[1].ends_with('…'), "{closed:?}");
        let open = texts(&HashSet::from(["t1".to_string()]));
        let text: String = open[2..]
            .iter()
            .map(|r| r.trim())
            .collect::<Vec<_>>()
            .join(" ");
        assert_eq!(text, body, "nothing is lost: {open:?}");
        assert!(open[2..].iter().all(|r| r.starts_with("    ")), "{open:?}");
        assert!(
            open.iter()
                .all(|r| unicode_width::UnicodeWidthStr::width(r.as_str()) <= 30)
        );
    }

    #[test]
    fn thread_rows_count_once_per_line_even_when_wrapped() {
        let text = Rope::from("one\ntwo\nthree\n");
        let mut t = Threads::default();
        for line in [0, 2, 3] {
            t.rows.entry(line).or_default().push(ThreadRow {
                text: String::new(),
                style: Style::default(),
                thread: 0,
            });
        }
        let mut a = ThreadLines::new(&t, text.slice(..));
        // After line 0's newline (char 4) one row; mid-line (a wrap) none; line 3 is the last.
        assert_eq!(a.insert_virtual_lines(4, Position::new(0, 0), 0).row, 1);
        assert_eq!(a.insert_virtual_lines(2, Position::new(0, 0), 0).row, 0);
        assert_eq!(a.insert_virtual_lines(14, Position::new(0, 0), 2).row, 1);
        assert_eq!(a.0.len(), 2);
    }

    #[test]
    fn external_editor_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("ed");
        std::fs::write(&script, "#!/bin/sh\nprintf ' world\\n' >> \"$1\"\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let ed = script.to_string_lossy();
        assert_eq!(external_edit(&ed, "hello").unwrap(), "hello world");
        assert!(external_edit("false", "x").is_err());
    }
}
