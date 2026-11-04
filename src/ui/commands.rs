// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at
// https://mozilla.org/MPL/2.0/.
//
// Ported from helix-term/src/commands.rs at Helix 25.07.1 (https://github.com/helix-editor/helix).
// The movement, selection and text-object logic itself is helix-core's, called here; these are
// the thin wrappers Helix keeps in helix-term, adapted to glos's read-only view.

//! Helix's normal- and select-mode commands, as functions over a text and its selection.

use std::num::NonZeroUsize;

use helix_core::{
    Range, Rope, Selection, doc_formatter::TextFormat, graphemes, line_ending::line_end_char_index,
    match_brackets, movement, movement::{Direction, Movement}, search,
    text_annotations::TextAnnotations, textobject::{self, TextObject},
};
use helix_stdx::rope::RopeSliceExt;

use crate::diff::Hunk;

/// What a command sees and changes: one file's text and selection, and the view over it.
pub struct Ctx<'a> {
    pub text: &'a Rope,
    pub sel: Selection,
    pub count: Option<NonZeroUsize>,
    /// Select mode: motions extend the selection.
    pub extend: bool,
    pub hunks: &'a [Hunk],
    /// The first line shown, and how many lines fit.
    pub top: usize,
    pub height: usize,
    pub fmt: TextFormat,
    pub status: Option<String>,
}

/// Lines kept between the cursor and the edge of the view, as Helix's default `scrolloff`.
pub const SCROLLOFF: usize = 5;

impl Ctx<'_> {
    fn count(&self) -> usize {
        self.count.map_or(1, NonZeroUsize::get)
    }

    fn movement(&self) -> Movement {
        if self.extend { Movement::Extend } else { Movement::Move }
    }

    fn transform(&mut self, f: impl FnMut(Range) -> Range) {
        self.sel = self.sel.clone().transform(f);
    }

    fn scrolloff(&self) -> usize {
        SCROLLOFF.min(self.height.saturating_sub(1) / 2)
    }

    /// The last line worth jumping to: a final empty line (after the last newline) isn't.
    fn last_line(&self) -> usize {
        let t = self.text.slice(..);
        if t.line(t.len_lines() - 1).len_chars() == 0 {
            t.len_lines().saturating_sub(2)
        } else {
            t.len_lines() - 1
        }
    }
}

type MoveFn = fn(
    helix_core::RopeSlice, Range, Direction, usize, Movement, &TextFormat, &mut TextAnnotations,
) -> Range;

fn move_impl(cx: &mut Ctx, f: MoveFn, dir: Direction) {
    let (text, count, behaviour, fmt) = (cx.text.slice(..), cx.count(), cx.movement(), cx.fmt.clone());
    let mut annotations = TextAnnotations::default();
    cx.transform(|r| f(text, r, dir, count, behaviour, &fmt, &mut annotations));
}

pub fn move_char_left(cx: &mut Ctx) {
    move_impl(cx, movement::move_horizontally, Direction::Backward)
}
pub fn move_char_right(cx: &mut Ctx) {
    move_impl(cx, movement::move_horizontally, Direction::Forward)
}
pub fn move_visual_line_up(cx: &mut Ctx) {
    move_impl(cx, movement::move_vertically_visual, Direction::Backward)
}
pub fn move_visual_line_down(cx: &mut Ctx) {
    move_impl(cx, movement::move_vertically_visual, Direction::Forward)
}

fn word_impl(cx: &mut Ctx, f: fn(helix_core::RopeSlice, Range, usize) -> Range) {
    let (text, count, extend) = (cx.text.slice(..), cx.count(), cx.extend);
    cx.transform(|r| {
        let word = f(text, r, count);
        if extend { r.put_cursor(text, word.cursor(text), true) } else { word }
    });
}

pub fn move_next_word_start(cx: &mut Ctx) {
    word_impl(cx, movement::move_next_word_start)
}
pub fn move_prev_word_start(cx: &mut Ctx) {
    word_impl(cx, movement::move_prev_word_start)
}
pub fn move_next_word_end(cx: &mut Ctx) {
    word_impl(cx, movement::move_next_word_end)
}
pub fn move_next_long_word_start(cx: &mut Ctx) {
    word_impl(cx, movement::move_next_long_word_start)
}
pub fn move_prev_long_word_start(cx: &mut Ctx) {
    word_impl(cx, movement::move_prev_long_word_start)
}
pub fn move_next_long_word_end(cx: &mut Ctx) {
    word_impl(cx, movement::move_next_long_word_end)
}

pub fn goto_line_start(cx: &mut Ctx) {
    let (text, extend) = (cx.text.slice(..), cx.extend);
    cx.transform(|r| r.put_cursor(text, text.line_to_char(r.cursor_line(text)), extend));
}

pub fn goto_line_end(cx: &mut Ctx) {
    let (text, extend) = (cx.text.slice(..), cx.extend);
    cx.transform(|r| {
        let line = r.cursor_line(text);
        let start = text.line_to_char(line);
        let pos = graphemes::prev_grapheme_boundary(text, line_end_char_index(&text, line)).max(start);
        r.put_cursor(text, pos, extend)
    });
}

pub fn goto_first_nonwhitespace(cx: &mut Ctx) {
    let (text, extend) = (cx.text.slice(..), cx.extend);
    cx.transform(|r| {
        let line = r.cursor_line(text);
        match text.line(line).first_non_whitespace_char() {
            Some(pos) => r.put_cursor(text, pos + text.line_to_char(line), extend),
            None => r,
        }
    });
}

fn goto_line_impl(cx: &mut Ctx, line: usize) {
    let (text, extend) = (cx.text.slice(..), cx.extend);
    let pos = text.line_to_char(line.min(cx.last_line()));
    cx.transform(|r| r.put_cursor(text, pos, extend));
}

/// `gg`, or with a count, that line.
pub fn goto_file_start(cx: &mut Ctx) {
    match cx.count {
        Some(n) => goto_line_impl(cx, n.get() - 1),
        None => goto_line_impl(cx, 0),
    }
}

/// `G`: the line given by the count.
pub fn goto_line(cx: &mut Ctx) {
    if let Some(n) = cx.count {
        goto_line_impl(cx, n.get() - 1);
    }
}

pub fn goto_last_line(cx: &mut Ctx) {
    let line = cx.last_line();
    goto_line_impl(cx, line);
}

#[derive(Clone, Copy)]
enum Align {
    Top,
    Center,
    Bottom,
}

fn goto_window(cx: &mut Ctx, align: Align) {
    let count = cx.count() - 1;
    let scrolloff = cx.scrolloff();
    let last_visual = cx.height.saturating_sub(1).min(cx.text.len_lines().saturating_sub(cx.top + 1));
    let line = match align {
        Align::Top => cx.top + scrolloff + count,
        Align::Center => cx.top + last_visual / 2,
        Align::Bottom => cx.top + last_visual.saturating_sub(scrolloff + count),
    }
    .max(cx.top + scrolloff)
    .min(cx.top + last_visual.saturating_sub(scrolloff));
    let line = line.min(cx.text.len_lines() - 1);
    let (text, extend) = (cx.text.slice(..), cx.extend);
    let pos = text.line_to_char(line);
    cx.transform(|r| r.put_cursor(text, pos, extend));
}

pub fn goto_window_top(cx: &mut Ctx) {
    goto_window(cx, Align::Top)
}
pub fn goto_window_center(cx: &mut Ctx) {
    goto_window(cx, Align::Center)
}
pub fn goto_window_bottom(cx: &mut Ctx) {
    goto_window(cx, Align::Bottom)
}

/// Scrolls the view by `offset` lines; with `sync_cursor` the cursor moves as far, otherwise it
/// is only kept inside the view (Helix's `scroll`).
fn scroll(cx: &mut Ctx, offset: usize, dir: Direction, sync_cursor: bool) {
    let last = cx.text.len_lines().saturating_sub(1);
    cx.top = match dir {
        Direction::Forward => (cx.top + offset).min(last),
        Direction::Backward => cx.top.saturating_sub(offset),
    };
    if sync_cursor {
        let (text, behaviour, fmt) = (cx.text.slice(..), cx.movement(), cx.fmt.clone());
        let mut annotations = TextAnnotations::default();
        cx.transform(|r| movement::move_vertically_visual(text, r, dir, offset, behaviour, &fmt, &mut annotations));
        return;
    }
    let scrolloff = cx.scrolloff();
    let (text, extend) = (cx.text.slice(..), cx.extend);
    let lo = (cx.top + scrolloff).min(last);
    let hi = (cx.top + cx.height.saturating_sub(1 + scrolloff)).min(last).max(lo);
    cx.transform(|r| {
        let line = r.cursor_line(text);
        if line < lo {
            r.put_cursor(text, text.line_to_char(lo), extend)
        } else if line > hi {
            r.put_cursor(text, text.line_to_char(hi), extend)
        } else {
            r
        }
    });
}

pub fn page_down(cx: &mut Ctx) {
    scroll(cx, cx.height, Direction::Forward, false)
}
pub fn page_up(cx: &mut Ctx) {
    scroll(cx, cx.height, Direction::Backward, false)
}
pub fn page_cursor_half_down(cx: &mut Ctx) {
    scroll(cx, cx.height / 2, Direction::Forward, true)
}
pub fn page_cursor_half_up(cx: &mut Ctx) {
    scroll(cx, cx.height / 2, Direction::Backward, true)
}

pub fn extend_line_below(cx: &mut Ctx) {
    let (text, count) = (cx.text, cx.count());
    cx.transform(|r| {
        let (start_line, end_line) = r.line_range(text.slice(..));
        let start = text.line_to_char(start_line);
        let end = text.line_to_char((end_line + 1).min(text.len_lines()));
        // Extend to the next line if the current one is already selected.
        let extra = if r.from() == start && r.to() == end { count } else { count - 1 };
        Range::new(start, text.line_to_char((end_line + 1 + extra).min(text.len_lines())))
    });
}

pub fn extend_to_line_bounds(cx: &mut Ctx) {
    let text = cx.text;
    cx.transform(|r| {
        let (start_line, end_line) = r.line_range(text.slice(..));
        let start = text.line_to_char(start_line);
        let end = text.line_to_char((end_line + 1).min(text.len_lines()));
        Range::new(start, end).with_direction(r.direction())
    });
}

pub fn select_all(cx: &mut Ctx) {
    cx.sel = Selection::single(0, cx.text.len_chars());
}

pub fn collapse_selection(cx: &mut Ctx) {
    let text = cx.text.slice(..);
    cx.transform(|r| {
        let pos = r.cursor(text);
        Range::new(pos, pos)
    });
}

pub fn flip_selections(cx: &mut Ctx) {
    cx.transform(|r| r.flip());
}

pub fn keep_primary_selection(cx: &mut Ctx) {
    let r = cx.sel.primary();
    cx.sel = Selection::single(r.anchor, r.head);
}

pub fn match_brackets(cx: &mut Ctx) {
    let (text, extend) = (cx.text.slice(..), cx.extend);
    // ponytail: plaintext matching; Helix uses the syntax tree when there is one.
    cx.transform(|r| match match_brackets::find_matching_bracket_plaintext(text, r.cursor(text)) {
        Some(pos) => r.put_cursor(text, pos, extend),
        None => r,
    });
}

/// `mi<ch>` / `ma<ch>`. The tree-sitter objects (f, t, a, c, T, e) are not supported yet.
pub fn select_textobject(cx: &mut Ctx, around: bool, ch: char) {
    let objtype = if around { TextObject::Around } else { TextObject::Inside };
    let (text, count, hunks) = (cx.text.slice(..), cx.count(), cx.hunks);
    cx.transform(|r| match ch {
        'w' => textobject::textobject_word(text, r, objtype, count, false),
        'W' => textobject::textobject_word(text, r, objtype, count, true),
        'p' => textobject::textobject_paragraph(text, r, objtype, count),
        'm' => textobject::textobject_pair_surround_closest(None, text, r, objtype, count),
        'g' => {
            let line = r.cursor_line(text) as u32;
            match hunks.iter().find(|h| h.after.start <= line && line < h.after.end.max(h.after.start + 1)) {
                Some(h) => Range::new(text.line_to_char(h.after.start as usize), text.line_to_char(h.after.end as usize))
                    .with_direction(r.direction()),
                None => r,
            }
        }
        ch if !ch.is_ascii_alphanumeric() => {
            textobject::textobject_pair_surround(None, text, r, objtype, ch, count)
        }
        _ => r,
    });
}

pub const TEXTOBJECT_HELP: &[(&str, &str)] = &[
    ("w", "Word"),
    ("W", "WORD"),
    ("p", "Paragraph"),
    ("m", "Closest surrounding pair"),
    ("g", "Change"),
    (" ", "... or any character acting as a pair"),
];

fn find_char_impl(cx: &mut Ctx, ch: char, forward: bool, inclusive: bool) {
    let (text, count, extend) = (cx.text.slice(..), cx.count(), cx.extend);
    cx.transform(|r| {
        // Helix works in chars here, not graphemes.
        let start = if r.anchor < r.head { r.head - 1 } else { r.head };
        let found = if forward {
            let pos = (start + 1).min(text.len_chars());
            if inclusive {
                search::find_nth_next(text, ch, pos, count)
            } else {
                let n = if text.get_char(pos) == Some(ch) { count + 1 } else { count };
                search::find_nth_next(text, ch, pos, n).map(|p| p.saturating_sub(1))
            }
        } else if inclusive {
            search::find_nth_prev(text, ch, start, count)
        } else {
            let n = if text.get_char(start.saturating_sub(1)) == Some(ch) { count + 1 } else { count };
            search::find_nth_prev(text, ch, start, n).map(|p| (p + 1).min(text.len_chars()))
        };
        match found {
            Some(pos) if extend => r.put_cursor(text, pos, true),
            Some(pos) => Range::point(r.cursor(text)).put_cursor(text, pos, true),
            None => r,
        }
    });
}

pub fn find_next_char(cx: &mut Ctx, ch: char) {
    find_char_impl(cx, ch, true, true)
}
pub fn find_till_char(cx: &mut Ctx, ch: char) {
    find_char_impl(cx, ch, true, false)
}
pub fn find_prev_char(cx: &mut Ctx, ch: char) {
    find_char_impl(cx, ch, false, true)
}
pub fn till_prev_char(cx: &mut Ctx, ch: char) {
    find_char_impl(cx, ch, false, false)
}

/// The hunk after `line` (helix-vcs `next_hunk`).
fn next_hunk(hunks: &[Hunk], line: u32) -> Option<usize> {
    match hunks.binary_search_by_key(&line, |h| h.after.start) {
        Ok(pos) if pos + 1 == hunks.len() => None,
        Ok(pos) => Some(pos + 1),
        Err(pos) if pos == hunks.len() => None,
        Err(pos) => Some(pos),
    }
}

/// The hunk before `line` (helix-vcs `prev_hunk`).
fn prev_hunk(hunks: &[Hunk], line: u32) -> Option<usize> {
    match hunks.binary_search_by_key(&line, |h| h.after.end) {
        Ok(pos) if !hunks[pos].after.is_empty() => Some(pos),
        Err(0) | Ok(0) => None,
        Err(pos) | Ok(pos) => Some(pos - 1),
    }
}

fn hunk_range(h: &Hunk, text: helix_core::RopeSlice) -> Range {
    let anchor = text.line_to_char(h.after.start as usize);
    let head = if h.after.is_empty() { anchor + 1 } else { text.line_to_char(h.after.end as usize) };
    Range::new(anchor, head)
}

fn goto_change(cx: &mut Ctx, dir: Direction) {
    if cx.hunks.is_empty() {
        cx.status = Some("No changes in this file".into());
        return;
    }
    let (text, extra, hunks, extend) = (cx.text.slice(..), cx.count() - 1, cx.hunks, cx.extend);
    cx.transform(|r| {
        let line = r.cursor_line(text) as u32;
        let idx = match dir {
            Direction::Forward => next_hunk(hunks, line).map(|i| (i + extra).min(hunks.len() - 1)),
            Direction::Backward => prev_hunk(hunks, line).map(|i| i.saturating_sub(extra)),
        };
        let Some(idx) = idx else { return r };
        let new = hunk_range(&hunks[idx], text);
        if extend {
            let head = if new.head < r.anchor { new.anchor } else { new.head };
            Range::new(r.anchor, head)
        } else {
            new.with_direction(dir)
        }
    });
}

pub fn goto_next_change(cx: &mut Ctx) {
    goto_change(cx, Direction::Forward)
}
pub fn goto_prev_change(cx: &mut Ctx) {
    goto_change(cx, Direction::Backward)
}

fn goto_nth_change(cx: &mut Ctx, idx: usize) {
    let Some(h) = cx.hunks.get(idx.min(cx.hunks.len().saturating_sub(1))) else {
        cx.status = Some("No changes in this file".into());
        return;
    };
    let new = hunk_range(h, cx.text.slice(..));
    cx.sel = Selection::single(new.anchor, new.head);
}

pub fn goto_first_change(cx: &mut Ctx) {
    goto_nth_change(cx, 0)
}
pub fn goto_last_change(cx: &mut Ctx) {
    goto_nth_change(cx, usize::MAX)
}

#[cfg(test)]
mod tests {
    //! In Helix's notation: `#[h|]#ello` is a selection with the cursor on `h`.
    use super::*;
    use helix_core::test::{plain, print};

    fn run_with(input: &str, extend: bool, hunks: &[Hunk], cmds: &[&dyn Fn(&mut Ctx)]) -> String {
        let (text, sel) = print(input);
        let rope = Rope::from(text);
        let mut cx = Ctx {
            text: &rope,
            sel,
            count: None,
            extend,
            hunks,
            top: 0,
            height: 40,
            fmt: TextFormat::default(),
            status: None,
        };
        for c in cmds {
            c(&mut cx);
            cx.sel = cx.sel.clone().ensure_invariants(rope.slice(..));
        }
        plain(rope.clone(), &cx.sel)
    }

    fn run(input: &str, cmds: &[&dyn Fn(&mut Ctx)]) -> String {
        run_with(input, false, &[], cmds)
    }

    fn counted(n: usize, f: fn(&mut Ctx)) -> impl Fn(&mut Ctx) {
        move |cx| {
            cx.count = NonZeroUsize::new(n);
            f(cx);
            cx.count = None;
        }
    }

    #[test]
    fn words() {
        assert_eq!(run("#[h|]#ello world", &[&move_next_word_start]), "#[hello |]#world");
        assert_eq!(run("#[h|]#ello world foo", &[&counted(2, move_next_word_start)]), "hello #[world |]#foo");
        assert_eq!(run("#[h|]#ello world", &[&move_next_word_end]), "#[hello|]# world");
        assert_eq!(run("hello wor#[l|]#d", &[&move_prev_word_start]), "hello #[|worl]#d");
        assert_eq!(run("#[a|]#.b c", &[&move_next_long_word_start]), "#[a.b |]#c");
    }

    #[test]
    fn select_mode_extends() {
        let w: &dyn Fn(&mut Ctx) = &move_next_word_start;
        assert_eq!(run_with("#[h|]#ello world foo", true, &[], &[w, w]), "#[hello world |]#foo");
        let l: &dyn Fn(&mut Ctx) = &move_char_right;
        assert_eq!(run_with("#[a|]#bc", true, &[], &[l, l]), "#[abc|]#");
    }

    #[test]
    fn chars_and_lines() {
        assert_eq!(run("a#[b|]#c\ndef", &[&move_visual_line_down]), "abc\nd#[e|]#f");
        assert_eq!(run("a#[b|]#c", &[&move_char_left]), "#[a|]#bc");
        assert_eq!(run("  a#[b|]#c\n", &[&goto_line_start]), "#[ |]# abc\n");
        assert_eq!(run("  a#[b|]#c\n", &[&goto_first_nonwhitespace]), "  #[a|]#bc\n");
        assert_eq!(run("#[a|]#bc\n", &[&goto_line_end]), "ab#[c|]#\n");
        assert_eq!(run("a\nb\n#[c|]#\n", &[&goto_file_start]), "#[a|]#\nb\nc\n");
        assert_eq!(run("#[a|]#\nb\nc\n", &[&goto_last_line]), "a\nb\n#[c|]#\n");
        assert_eq!(run("#[a|]#\nb\nc\n", &[&counted(2, goto_line)]), "a\n#[b|]#\nc\n");
    }

    #[test]
    fn line_selection() {
        let t = "a#[b|]#c\ndef\nghi\n";
        assert_eq!(run(t, &[&extend_line_below]), "#[abc\n|]#def\nghi\n");
        assert_eq!(run(t, &[&extend_line_below, &extend_line_below]), "#[abc\ndef\n|]#ghi\n");
        assert_eq!(run(t, &[&counted(2, extend_line_below)]), "#[abc\ndef\n|]#ghi\n");
        assert_eq!(run("a#[b|]#c\ndef\n", &[&extend_to_line_bounds]), "#[abc\n|]#def\n");
    }

    #[test]
    fn selections() {
        assert_eq!(run("a#[b|]#c", &[&select_all]), "#[abc|]#");
        assert_eq!(run("#[abc|]#", &[&collapse_selection]), "ab#[c|]#");
        assert_eq!(run("#[abc|]#", &[&flip_selections]), "#[|abc]#");
        assert_eq!(run("#[a|]#a\n#(b|)#b\n", &[&keep_primary_selection]), "#[a|]#a\nbb\n");
        assert_eq!(run("#[a|]#a bb\n#(c|)#c dd\n", &[&move_next_word_start]), "#[aa |]#bb\n#(cc |)#dd\n");
        assert_eq!(run("f#[(|]#a, b)", &[&match_brackets]), "f(a, b#[)|]#");
    }

    #[test]
    fn textobjects() {
        assert_eq!(run("foo(a, #[b|]#)", &[&|cx| select_textobject(cx, false, '(')]), "foo(#[a, b|]#)");
        assert_eq!(run("foo(a, #[b|]#)", &[&|cx| select_textobject(cx, true, '(')]), "foo#[(a, b)|]#");
        assert_eq!(run("one tw#[o|]# three", &[&|cx| select_textobject(cx, false, 'w')]), "one #[two|]# three");
        let hunks = [Hunk { before: 1..1, after: 1..3 }];
        assert_eq!(
            run_with("a\nb#[b|]#\nc\nd\n", false, &hunks, &[&|cx| select_textobject(cx, false, 'g')]),
            "a\n#[bb\nc\n|]#d\n"
        );
    }

    #[test]
    fn find_char() {
        assert_eq!(run("#[a|]#bcabc", &[&|cx| find_next_char(cx, 'c')]), "#[abc|]#abc");
        assert_eq!(run("#[a|]#bcabc", &[&|cx| find_till_char(cx, 'c')]), "#[ab|]#cabc");
        assert_eq!(run("abcab#[c|]#", &[&|cx| find_prev_char(cx, 'a')]), "abc#[|abc]#");
        assert_eq!(run("abcab#[c|]#", &[&|cx| till_prev_char(cx, 'a')]), "abca#[|bc]#");
    }

    #[test]
    fn changes() {
        // Hunks on lines 1–2 and 4 (a pure removal before line 5 is empty: 5..5).
        let hunks = [Hunk { before: 1..1, after: 1..3 }, Hunk { before: 3..5, after: 4..4 }];
        let t = "#[a|]#\nb\nc\nd\ne\nf\n";
        assert_eq!(run_with(t, false, &hunks, &[&goto_next_change]), "a\n#[b\nc\n|]#d\ne\nf\n");
        assert_eq!(
            run_with(t, false, &hunks, &[&goto_next_change, &goto_next_change]),
            "a\nb\nc\nd\n#[e|]#\nf\n"
        );
        assert_eq!(run_with("a\nb\nc\nd\ne\n#[f|]#\n", false, &hunks, &[&goto_prev_change]), "a\nb\nc\nd\n#[|e]#\nf\n");
        assert_eq!(run_with(t, false, &hunks, &[&goto_last_change]), "a\nb\nc\nd\n#[e|]#\nf\n");
    }
}
