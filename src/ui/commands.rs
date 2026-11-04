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
    Range, Rope, Selection,
    doc_formatter::TextFormat,
    graphemes,
    line_ending::line_end_char_index,
    match_brackets, movement,
    movement::{Direction, Movement},
    search,
    text_annotations::TextAnnotations,
    textobject::{self, TextObject},
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
    /// The first line and column shown, and how many lines fit.
    pub top: usize,
    pub left: usize,
    pub height: usize,
    pub fmt: TextFormat,
    /// A message for the command line; `true` for an error.
    pub status: Option<(String, bool)>,
    /// Centre the cursor if it ends up outside the view (Helix's `ensure_cursor_in_view_center`).
    pub center: bool,
    /// The command placed the view itself (`zz`…): don't scroll to the cursor afterwards.
    pub keep_top: bool,
}

/// Lines kept between the cursor and the edge of the view, as Helix's default `scrolloff`.
pub const SCROLLOFF: usize = 5;

impl Ctx<'_> {
    fn count(&self) -> usize {
        self.count.map_or(1, NonZeroUsize::get)
    }

    fn movement(&self) -> Movement {
        if self.extend {
            Movement::Extend
        } else {
            Movement::Move
        }
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
    helix_core::RopeSlice,
    Range,
    Direction,
    usize,
    Movement,
    &TextFormat,
    &mut TextAnnotations,
) -> Range;

fn move_impl(cx: &mut Ctx, f: MoveFn, dir: Direction) {
    let (text, count, behaviour, fmt) =
        (cx.text.slice(..), cx.count(), cx.movement(), cx.fmt.clone());
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
        if extend {
            r.put_cursor(text, word.cursor(text), true)
        } else {
            word
        }
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
        let pos =
            graphemes::prev_grapheme_boundary(text, line_end_char_index(&text, line)).max(start);
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
    let last_visual = cx
        .height
        .saturating_sub(1)
        .min(cx.text.len_lines().saturating_sub(cx.top + 1));
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
        cx.transform(|r| {
            movement::move_vertically_visual(
                text,
                r,
                dir,
                offset,
                behaviour,
                &fmt,
                &mut annotations,
            )
        });
        return;
    }
    // Otherwise only the primary cursor moves, to the edge of the scrolloff zone if it fell
    // outside (Helix's `scroll`).
    let scrolloff = cx.scrolloff();
    let text = cx.text.slice(..);
    let annotations = TextAnnotations::default();
    let anchor = text.line_to_char(cx.top);
    let range = cx.sel.primary();
    let cursor = range.cursor(text);
    let head = match dir {
        Direction::Forward => {
            let (head, off) = helix_core::char_idx_at_visual_offset(
                text,
                anchor,
                scrolloff as isize,
                0,
                &cx.fmt,
                &annotations,
            );
            let head = head + (off != 0) as usize;
            if head <= cursor {
                return;
            }
            head
        }
        Direction::Backward => {
            let row = cx.height.saturating_sub(scrolloff + 1) as isize;
            let (head, _) =
                helix_core::char_idx_at_visual_offset(text, anchor, row, 0, &cx.fmt, &annotations);
            if head >= cursor {
                return;
            }
            head
        }
    };
    let anchor = if cx.extend { range.anchor } else { head };
    cx.sel = cx
        .sel
        .clone()
        .replace(cx.sel.primary_index(), Range::new(anchor, head));
}

/// The mouse wheel: the view moves, and the cursor only as far as it must to stay in view.
pub fn scroll_lines(cx: &mut Ctx, lines: usize, down: bool) {
    let dir = if down {
        Direction::Forward
    } else {
        Direction::Backward
    };
    scroll(cx, lines, dir, false)
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
        let extra = if r.from() == start && r.to() == end {
            count
        } else {
            count - 1
        };
        Range::new(
            start,
            text.line_to_char((end_line + 1 + extra).min(text.len_lines())),
        )
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
    cx.transform(
        |r| match match_brackets::find_matching_bracket_plaintext(text, r.cursor(text)) {
            Some(pos) => r.put_cursor(text, pos, extend),
            None => r,
        },
    );
}

/// `mi<ch>` / `ma<ch>`. The tree-sitter objects (f, t, a, c, T, e) are not supported yet.
pub fn select_textobject(cx: &mut Ctx, around: bool, ch: char) {
    let objtype = if around {
        TextObject::Around
    } else {
        TextObject::Inside
    };
    let (text, count, hunks) = (cx.text.slice(..), cx.count(), cx.hunks);
    cx.transform(|r| match ch {
        'w' => textobject::textobject_word(text, r, objtype, count, false),
        'W' => textobject::textobject_word(text, r, objtype, count, true),
        'p' => textobject::textobject_paragraph(text, r, objtype, count),
        'm' => textobject::textobject_pair_surround_closest(None, text, r, objtype, count),
        'g' => {
            let line = r.cursor_line(text) as u32;
            match hunks
                .iter()
                .find(|h| h.after.start <= line && line < h.after.end.max(h.after.start + 1))
            {
                Some(h) => Range::new(
                    text.line_to_char(h.after.start as usize),
                    text.line_to_char(h.after.end as usize),
                )
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
        let start = if r.anchor < r.head {
            r.head - 1
        } else {
            r.head
        };
        let found = if forward {
            let pos = (start + 1).min(text.len_chars());
            if inclusive {
                search::find_nth_next(text, ch, pos, count)
            } else {
                let n = if text.get_char(pos) == Some(ch) {
                    count + 1
                } else {
                    count
                };
                search::find_nth_next(text, ch, pos, n).map(|p| p.saturating_sub(1))
            }
        } else if inclusive {
            search::find_nth_prev(text, ch, start, count)
        } else {
            let n = if text.get_char(start.saturating_sub(1)) == Some(ch) {
                count + 1
            } else {
                count
            };
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
    let head = if h.after.is_empty() {
        anchor + 1
    } else {
        text.line_to_char(h.after.end as usize)
    };
    Range::new(anchor, head)
}

fn goto_change(cx: &mut Ctx, dir: Direction) {
    if cx.hunks.is_empty() {
        cx.status = Some(("No changes in this file".into(), false));
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
            let head = if new.head < r.anchor {
                new.anchor
            } else {
                new.head
            };
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
        cx.status = Some(("No changes in this file".into(), false));
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

pub fn move_line_up(cx: &mut Ctx) {
    move_impl(cx, movement::move_vertically, Direction::Backward)
}
pub fn move_line_down(cx: &mut Ctx) {
    move_impl(cx, movement::move_vertically, Direction::Forward)
}

/// `g|`: column `count` on the line.
pub fn goto_column(cx: &mut Ctx) {
    let (text, count, extend) = (cx.text.slice(..), cx.count(), cx.extend);
    cx.transform(|r| {
        let line = r.cursor_line(text);
        let start = text.line_to_char(line);
        let pos = graphemes::nth_next_grapheme_boundary(text, start, count - 1)
            .min(line_end_char_index(&text, line));
        r.put_cursor(text, pos, extend)
    });
}

fn para_impl(cx: &mut Ctx, f: fn(helix_core::RopeSlice, Range, usize, Movement) -> Range) {
    let (text, count, behaviour) = (cx.text.slice(..), cx.count(), cx.movement());
    cx.transform(|r| f(text, r, count, behaviour));
}
pub fn goto_next_paragraph(cx: &mut Ctx) {
    para_impl(cx, movement::move_next_paragraph)
}
pub fn goto_prev_paragraph(cx: &mut Ctx) {
    para_impl(cx, movement::move_prev_paragraph)
}

/// `C` / `A-C`: copies each selection to the next (or previous) line where it fits. Helix
/// itself still uses these deprecated column helpers here.
#[allow(deprecated)]
fn copy_selection_on_line(cx: &mut Ctx, dir: Direction) {
    use helix_core::{Position, pos_at_visual_coords, visual_coords_at_pos};
    let (text, count, tab) = (cx.text.slice(..), cx.count(), cx.fmt.tab_width as usize);
    let sel = &cx.sel;
    let mut ranges: Vec<Range> = sel.ranges().to_vec();
    let mut primary_index = 0;
    for range in sel.iter() {
        let is_primary = *range == sel.primary();
        // The range is always head exclusive.
        let (head, anchor) = if range.anchor < range.head {
            (range.head - 1, range.anchor)
        } else {
            (range.head, range.anchor.saturating_sub(1))
        };
        let (head_pos, anchor_pos) = (
            visual_coords_at_pos(text, head, tab),
            visual_coords_at_pos(text, anchor, tab),
        );
        let height = head_pos.row.max(anchor_pos.row) - head_pos.row.min(anchor_pos.row) + 1;
        if is_primary {
            primary_index = ranges.len();
        }
        ranges.push(*range);
        let (mut sels, mut i) = (0, 0);
        while sels < count {
            let offset = (i + 1) * height;
            let (anchor_row, head_row) = match dir {
                Direction::Forward => (anchor_pos.row + offset, head_pos.row + offset),
                Direction::Backward => (
                    anchor_pos.row.saturating_sub(offset),
                    head_pos.row.saturating_sub(offset),
                ),
            };
            if anchor_row >= text.len_lines() || head_row >= text.len_lines() {
                break;
            }
            let (a, h) = (
                pos_at_visual_coords(text, Position::new(anchor_row, anchor_pos.col), tab),
                pos_at_visual_coords(text, Position::new(head_row, head_pos.col), tab),
            );
            // Skip lines that are too short.
            if visual_coords_at_pos(text, a, tab).col == anchor_pos.col
                && visual_coords_at_pos(text, h, tab).col == head_pos.col
            {
                if is_primary {
                    primary_index = ranges.len();
                }
                ranges.push(Range::point(a).put_cursor(text, h, true));
                sels += 1;
            }
            if anchor_row == 0 && head_row == 0 {
                break;
            }
            i += 1;
        }
    }
    cx.sel = Selection::new(ranges.into(), primary_index);
}
pub fn copy_selection_on_next_line(cx: &mut Ctx) {
    copy_selection_on_line(cx, Direction::Forward)
}
pub fn copy_selection_on_prev_line(cx: &mut Ctx) {
    copy_selection_on_line(cx, Direction::Backward)
}

pub fn trim_selections(cx: &mut Ctx) {
    let text = cx.text.slice(..);
    let ranges: Vec<Range> = cx
        .sel
        .iter()
        .filter_map(|r| {
            if r.is_empty() || r.slice(text).chars().all(char::is_whitespace) {
                return None;
            }
            let start =
                movement::skip_while(text, r.from(), |c| c.is_whitespace()).unwrap_or(r.from());
            let end = movement::backwards_skip_while(text, r.to(), |c| c.is_whitespace())
                .unwrap_or(r.to());
            Some(Range::new(start, end).with_direction(r.direction()))
        })
        .collect();
    if ranges.is_empty() {
        collapse_selection(cx);
        keep_primary_selection(cx);
    } else {
        let primary = cx.sel.primary();
        let idx = ranges
            .iter()
            .position(|r| r.overlaps(&primary))
            .unwrap_or(ranges.len() - 1);
        cx.sel = Selection::new(ranges.into(), idx);
    }
}

fn rotate_selections(cx: &mut Ctx, dir: Direction) {
    let (count, len, index) = (cx.count(), cx.sel.len(), cx.sel.primary_index());
    cx.sel.set_primary_index(match dir {
        Direction::Forward => (index + count) % len,
        Direction::Backward => (index + (len.saturating_sub(count) % len)) % len,
    });
}
pub fn rotate_selections_forward(cx: &mut Ctx) {
    rotate_selections(cx, Direction::Forward)
}
pub fn rotate_selections_backward(cx: &mut Ctx) {
    rotate_selections(cx, Direction::Backward)
}

pub fn merge_selections(cx: &mut Ctx) {
    cx.sel = cx.sel.clone().merge_ranges();
}
pub fn merge_consecutive_selections(cx: &mut Ctx) {
    cx.sel = cx.sel.clone().merge_consecutive_ranges();
}
pub fn ensure_selections_forward(cx: &mut Ctx) {
    cx.transform(|r| r.with_direction(Direction::Forward));
}

pub fn shrink_to_line_bounds(cx: &mut Ctx) {
    let text = cx.text;
    cx.transform(|r| {
        let (start_line, end_line) = r.line_range(text.slice(..));
        // Within one line there's nothing to shrink.
        if start_line == end_line {
            return r;
        }
        let mut start = text.line_to_char(start_line);
        let mut end = text.line_to_char((end_line + 1).min(text.len_lines()));
        if start != r.from() {
            start = text.line_to_char((start_line + 1).min(text.len_lines()));
        }
        if end != r.to() {
            end = text.line_to_char(end_line);
        }
        Range::new(start, end).with_direction(r.direction())
    });
}

pub fn remove_primary_selection(cx: &mut Ctx) {
    if cx.sel.len() == 1 {
        cx.status = Some(("no selections remaining".into(), true));
        return;
    }
    cx.sel = cx.sel.clone().remove(cx.sel.primary_index());
}

pub fn split_selection_on_newline(cx: &mut Ctx) {
    cx.sel = helix_core::selection::split_on_newline(cx.text.slice(..), &cx.sel);
}

/// What a regex prompt (`s`, `S`, `K`, `A-K`) does with the selections.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RegexAction {
    Select,
    Split,
    Keep,
    Remove,
}

impl RegexAction {
    pub fn prompt(self) -> &'static str {
        match self {
            RegexAction::Select => "select:",
            RegexAction::Split => "split:",
            RegexAction::Keep => "keep:",
            RegexAction::Remove => "remove:",
        }
    }
}

/// Helix's `select_regex`, `split_selection` and `keep_or_remove_selections_impl`.
pub fn regex_selections(cx: &mut Ctx, action: RegexAction, regex: &helix_stdx::rope::Regex) {
    use helix_core::selection;
    let text = cx.text.slice(..);
    let result = match action {
        RegexAction::Select => {
            selection::select_on_matches(text, &cx.sel, regex).ok_or("nothing selected")
        }
        RegexAction::Split => Ok(selection::split_on_matches(text, &cx.sel, regex)),
        RegexAction::Keep | RegexAction::Remove => {
            selection::keep_or_remove_matches(text, &cx.sel, regex, action == RegexAction::Remove)
                .ok_or("no selections remaining")
        }
    };
    match result {
        Ok(sel) => cx.sel = sel,
        Err(e) => cx.status = Some((e.into(), true)),
    }
}

/// `zz`, `zt`, `zb`: puts the cursor's line at the centre, top or bottom of the view.
fn align_view(cx: &mut Ctx, align: Align) {
    let text = cx.text.slice(..);
    let cursor = cx.sel.primary().cursor(text);
    let last = cx.height.saturating_sub(1);
    let relative = match align {
        Align::Center => last / 2,
        Align::Top => 0,
        Align::Bottom => last,
    };
    let (anchor, _) = helix_core::char_idx_at_visual_offset(
        text,
        cursor,
        -(relative as isize),
        0,
        &cx.fmt,
        &TextAnnotations::default(),
    );
    cx.top = text.char_to_line(anchor);
    cx.keep_top = true;
}
/// `zm`: the cursor's column to the middle of the view (no soft wrap only).
pub fn align_view_middle(cx: &mut Ctx) {
    if cx.fmt.soft_wrap {
        return;
    }
    let text = cx.text.slice(..);
    let col = super::view::visual_col(
        text,
        cx.sel.primary().cursor(text),
        cx.fmt.tab_width as usize,
    );
    cx.left = col.saturating_sub(cx.fmt.viewport_width as usize / 2);
    cx.keep_top = true;
}

pub fn align_view_center(cx: &mut Ctx) {
    align_view(cx, Align::Center)
}
pub fn align_view_top(cx: &mut Ctx) {
    align_view(cx, Align::Top)
}
pub fn align_view_bottom(cx: &mut Ctx) {
    align_view(cx, Align::Bottom)
}

/// `zk` / `zj`: scroll by `count` lines, keeping the cursor in view.
pub fn scroll_up(cx: &mut Ctx) {
    scroll(cx, cx.count(), Direction::Backward, false)
}
pub fn scroll_down(cx: &mut Ctx) {
    scroll(cx, cx.count(), Direction::Forward, false)
}

/// The search regex for `query`, with Helix's smart case: case-insensitive unless the query has
/// an uppercase letter.
pub fn search_regex(query: &str) -> Option<helix_stdx::rope::Regex> {
    let case_insensitive = !query.chars().any(char::is_uppercase);
    helix_stdx::rope::RegexBuilder::new()
        .syntax(
            helix_stdx::rope::Config::new()
                .case_insensitive(case_insensitive)
                .multi_line(true),
        )
        .build(query)
        .ok()
}

/// Selects the next match of `regex` after the primary selection (or before it, backwards),
/// wrapping around the file (Helix's `search_impl`). In select mode the match is added.
pub fn search(cx: &mut Ctx, regex: &helix_stdx::rope::Regex, forward: bool, show_warnings: bool) {
    let text = cx.text.slice(..);
    let primary = cx.sel.primary();
    let start = if forward {
        text.char_to_byte(graphemes::ensure_grapheme_boundary_next(text, primary.to()))
    } else {
        text.char_to_byte(graphemes::ensure_grapheme_boundary_prev(
            text,
            primary.from(),
        ))
    };
    let mut mat = if forward {
        regex.find(text.regex_input_at_bytes(start..))
    } else {
        regex.find_iter(text.regex_input_at_bytes(..start)).last()
    };
    if mat.is_none() {
        mat = if forward {
            regex.find(text.regex_input())
        } else {
            regex.find_iter(text.regex_input_at_bytes(start..)).last()
        };
        if show_warnings {
            cx.status = Some(match mat {
                Some(_) => ("Wrapped around document".into(), false),
                None => ("No more matches".into(), true),
            });
        }
    }
    let Some(mat) = mat else { return };
    let (start, end) = (text.byte_to_char(mat.start()), text.byte_to_char(mat.end()));
    if end == 0 {
        return; // an empty match at the start isn't worth selecting
    }
    let range = Range::new(start, end).with_direction(primary.direction());
    cx.sel = if cx.extend {
        cx.sel.clone().push(range)
    } else {
        cx.sel.clone().replace(cx.sel.primary_index(), range)
    };
    cx.center = true;
}

/// `n` / `N`: the last search again, `count` times.
pub fn search_next(cx: &mut Ctx, query: &str, forward: bool) {
    let Some(regex) = search_regex(query) else {
        return;
    };
    for _ in 0..cx.count() {
        search(cx, &regex, forward, true);
    }
}

/// `*` / `A-*`: the selections, escaped, become the search; with `detect_word_boundaries`
/// they get `\b` where they start or end a word (Helix's `search_selection_impl`).
pub fn search_selection(cx: &mut Ctx, detect_word_boundaries: bool) -> String {
    use helix_core::chars::char_is_word;
    let text = cx.text.slice(..);
    let at_start = |i: usize| {
        i < text.len_chars()
            && char_is_word(text.char(i))
            && (i == 0 || !char_is_word(text.char(i - 1)))
    };
    let at_end = |i: usize| {
        i > 0
            && i < text.len_chars()
            && char_is_word(text.char(i - 1))
            && !char_is_word(text.char(i))
    };
    let mut parts: Vec<String> = cx
        .sel
        .iter()
        .map(|r| {
            let pre = if detect_word_boundaries && at_start(r.from()) {
                "\\b"
            } else {
                ""
            };
            let post = if detect_word_boundaries && at_end(r.to()) {
                "\\b"
            } else {
                ""
            };
            format!("{pre}{}{post}", regex::escape(&r.fragment(text)))
        })
        .collect();
    parts.sort();
    parts.dedup();
    let query = parts.join("|");
    cx.status = Some((format!("register '/' set to '{query}'"), false));
    query
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
            left: 0,
            height: 40,
            fmt: TextFormat::default(),
            status: None,
            center: false,
            keep_top: false,
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
        assert_eq!(
            run("#[h|]#ello world", &[&move_next_word_start]),
            "#[hello |]#world"
        );
        assert_eq!(
            run("#[h|]#ello world foo", &[&counted(2, move_next_word_start)]),
            "hello #[world |]#foo"
        );
        assert_eq!(
            run("#[h|]#ello world", &[&move_next_word_end]),
            "#[hello|]# world"
        );
        assert_eq!(
            run("hello wor#[l|]#d", &[&move_prev_word_start]),
            "hello #[|worl]#d"
        );
        assert_eq!(
            run("#[a|]#.b c", &[&move_next_long_word_start]),
            "#[a.b |]#c"
        );
    }

    #[test]
    fn select_mode_extends() {
        let w: &dyn Fn(&mut Ctx) = &move_next_word_start;
        assert_eq!(
            run_with("#[h|]#ello world foo", true, &[], &[w, w]),
            "#[hello world |]#foo"
        );
        let l: &dyn Fn(&mut Ctx) = &move_char_right;
        assert_eq!(run_with("#[a|]#bc", true, &[], &[l, l]), "#[abc|]#");
    }

    #[test]
    fn chars_and_lines() {
        assert_eq!(
            run("a#[b|]#c\ndef", &[&move_visual_line_down]),
            "abc\nd#[e|]#f"
        );
        assert_eq!(run("a#[b|]#c", &[&move_char_left]), "#[a|]#bc");
        assert_eq!(run("  a#[b|]#c\n", &[&goto_line_start]), "#[ |]# abc\n");
        assert_eq!(
            run("  a#[b|]#c\n", &[&goto_first_nonwhitespace]),
            "  #[a|]#bc\n"
        );
        assert_eq!(run("#[a|]#bc\n", &[&goto_line_end]), "ab#[c|]#\n");
        assert_eq!(run("a\nb\n#[c|]#\n", &[&goto_file_start]), "#[a|]#\nb\nc\n");
        assert_eq!(run("#[a|]#\nb\nc\n", &[&goto_last_line]), "a\nb\n#[c|]#\n");
        assert_eq!(
            run("#[a|]#\nb\nc\n", &[&counted(2, goto_line)]),
            "a\n#[b|]#\nc\n"
        );
    }

    #[test]
    fn line_selection() {
        let t = "a#[b|]#c\ndef\nghi\n";
        assert_eq!(run(t, &[&extend_line_below]), "#[abc\n|]#def\nghi\n");
        assert_eq!(
            run(t, &[&extend_line_below, &extend_line_below]),
            "#[abc\ndef\n|]#ghi\n"
        );
        assert_eq!(
            run(t, &[&counted(2, extend_line_below)]),
            "#[abc\ndef\n|]#ghi\n"
        );
        assert_eq!(
            run("a#[b|]#c\ndef\n", &[&extend_to_line_bounds]),
            "#[abc\n|]#def\n"
        );
    }

    #[test]
    fn selections() {
        assert_eq!(run("a#[b|]#c", &[&select_all]), "#[abc|]#");
        assert_eq!(run("#[abc|]#", &[&collapse_selection]), "ab#[c|]#");
        assert_eq!(run("#[abc|]#", &[&flip_selections]), "#[|abc]#");
        assert_eq!(
            run("#[a|]#a\n#(b|)#b\n", &[&keep_primary_selection]),
            "#[a|]#a\nbb\n"
        );
        assert_eq!(
            run("#[a|]#a bb\n#(c|)#c dd\n", &[&move_next_word_start]),
            "#[aa |]#bb\n#(cc |)#dd\n"
        );
        assert_eq!(run("f#[(|]#a, b)", &[&match_brackets]), "f(a, b#[)|]#");
    }

    #[test]
    fn textobjects() {
        assert_eq!(
            run("foo(a, #[b|]#)", &[&|cx| select_textobject(cx, false, '(')]),
            "foo(#[a, b|]#)"
        );
        assert_eq!(
            run("foo(a, #[b|]#)", &[&|cx| select_textobject(cx, true, '(')]),
            "foo#[(a, b)|]#"
        );
        assert_eq!(
            run(
                "one tw#[o|]# three",
                &[&|cx| select_textobject(cx, false, 'w')]
            ),
            "one #[two|]# three"
        );
        let hunks = [Hunk {
            before: 1..1,
            after: 1..3,
        }];
        assert_eq!(
            run_with(
                "a\nb#[b|]#\nc\nd\n",
                false,
                &hunks,
                &[&|cx| select_textobject(cx, false, 'g')]
            ),
            "a\n#[bb\nc\n|]#d\n"
        );
    }

    #[test]
    fn find_char() {
        assert_eq!(
            run("#[a|]#bcabc", &[&|cx| find_next_char(cx, 'c')]),
            "#[abc|]#abc"
        );
        assert_eq!(
            run("#[a|]#bcabc", &[&|cx| find_till_char(cx, 'c')]),
            "#[ab|]#cabc"
        );
        assert_eq!(
            run("abcab#[c|]#", &[&|cx| find_prev_char(cx, 'a')]),
            "abc#[|abc]#"
        );
        assert_eq!(
            run("abcab#[c|]#", &[&|cx| till_prev_char(cx, 'a')]),
            "abca#[|bc]#"
        );
    }

    #[test]
    fn changes() {
        // Hunks on lines 1–2 and 4 (a pure removal before line 5 is empty: 5..5).
        let hunks = [
            Hunk {
                before: 1..1,
                after: 1..3,
            },
            Hunk {
                before: 3..5,
                after: 4..4,
            },
        ];
        let t = "#[a|]#\nb\nc\nd\ne\nf\n";
        assert_eq!(
            run_with(t, false, &hunks, &[&goto_next_change]),
            "a\n#[b\nc\n|]#d\ne\nf\n"
        );
        assert_eq!(
            run_with(t, false, &hunks, &[&goto_next_change, &goto_next_change]),
            "a\nb\nc\nd\n#[e|]#\nf\n"
        );
        assert_eq!(
            run_with(
                "a\nb\nc\nd\ne\n#[f|]#\n",
                false,
                &hunks,
                &[&goto_prev_change]
            ),
            "a\nb\nc\nd\n#[|e]#\nf\n"
        );
        assert_eq!(
            run_with(t, false, &hunks, &[&goto_last_change]),
            "a\nb\nc\nd\n#[e|]#\nf\n"
        );
    }

    #[test]
    fn searching() {
        let re = search_regex("b").unwrap();
        assert_eq!(
            run("#[a|]#b ab", &[&|cx| search(cx, &re, true, false)]),
            "a#[b|]# ab"
        );
        assert_eq!(
            run("a#[b|]# ab", &[&|cx| search(cx, &re, true, false)]),
            "ab a#[b|]#"
        );
        // Wraps around, and smart case: lowercase matches either case.
        assert_eq!(
            run("aB a#[b|]#", &[&|cx| search(cx, &re, true, false)]),
            "a#[B|]# ab"
        );
        assert_eq!(
            run("ab a#[b|]#", &[&|cx| search(cx, &re, false, false)]),
            "a#[b|]# ab"
        );
        let upper = search_regex("B").unwrap();
        assert_eq!(
            run("#[a|]#b aB", &[&|cx| search(cx, &upper, true, false)]),
            "ab a#[B|]#"
        );
        assert_eq!(
            run("#[a|]#b ab", &[&|cx| search_next(cx, "b", true)]),
            "a#[b|]# ab"
        );
    }

    #[test]
    fn star_detects_word_boundaries() {
        let (text, sel) = print("#[foo|]# foobar a.b");
        let rope = Rope::from(text);
        let mut cx = Ctx {
            text: &rope,
            sel,
            count: None,
            extend: false,
            hunks: &[],
            top: 0,
            left: 0,
            height: 10,
            fmt: TextFormat::default(),
            status: None,
            center: false,
            keep_top: false,
        };
        assert_eq!(search_selection(&mut cx, true), "\\bfoo\\b");
        assert_eq!(search_selection(&mut cx, false), "foo");
        cx.sel = Selection::single(11, 14);
        assert_eq!(search_selection(&mut cx, true), "\\ba\\.b");
    }

    #[test]
    fn selection_manipulation() {
        assert_eq!(
            run("#[a|]#b\ncd\n", &[&copy_selection_on_next_line]),
            "#(a|)#b\n#[c|]#d\n"
        );
        assert_eq!(run("#[  ab  |]#", &[&trim_selections]), "  #[ab|]#  ");
        assert_eq!(
            run("#[a|]# #(b|)#", &[&rotate_selections_forward]),
            "#(a|)# #[b|]#"
        );
        assert_eq!(
            run("#[a|]##(b|)#", &[&merge_consecutive_selections]),
            "#[ab|]#"
        );
        assert_eq!(run("#[|ab]#", &[&ensure_selections_forward]), "#[ab|]#");
        assert_eq!(
            run("#[a|]# #(b|)#", &[&remove_primary_selection]),
            "a #[b|]#"
        );
        assert_eq!(
            run("#[ab\ncd|]#", &[&split_selection_on_newline]),
            "#[ab|]#\n#(cd|)#"
        );
        let re = helix_stdx::rope::Regex::new("b").unwrap();
        assert_eq!(
            run(
                "#[abab|]#",
                &[&|cx| regex_selections(cx, RegexAction::Select, &re)]
            ),
            "a#[b|]#a#(b|)#"
        );
        assert_eq!(
            run(
                "#[a|]# #(b|)#",
                &[&|cx| regex_selections(cx, RegexAction::Keep, &re)]
            ),
            "a #[b|]#"
        );
        assert_eq!(
            run(
                "#[a|]# #(b|)#",
                &[&|cx| regex_selections(cx, RegexAction::Remove, &re)]
            ),
            "#[a|]# b"
        );
    }

    #[test]
    fn lines_columns_paragraphs() {
        assert_eq!(run("#[a|]#bc", &[&counted(3, goto_column)]), "ab#[c|]#");
        assert_eq!(
            run("#[a|]#\nb\n\nc\n", &[&goto_next_paragraph]),
            "#[a\nb\n\n|]#c\n"
        );
        // The column is kept, so a shorter line puts the cursor on its end, as in Helix.
        assert_eq!(run("ab#[c|]#\nd\n", &[&move_line_down]), "abc\nd#[\n|]#");
    }

    /// Runs one command over a view starting at line `top`, `height` lines tall; returns the
    /// selection and the new top.
    fn in_view(input: &str, top: usize, height: usize, f: fn(&mut Ctx)) -> (String, usize) {
        let (text, sel) = print(input);
        let rope = Rope::from(text);
        let mut cx = Ctx {
            text: &rope,
            sel,
            count: None,
            extend: false,
            hunks: &[],
            top,
            left: 0,
            height,
            fmt: TextFormat::default(),
            status: None,
            center: false,
            keep_top: false,
        };
        f(&mut cx);
        let top = cx.top;
        (
            plain(rope.clone(), &cx.sel.ensure_invariants(rope.slice(..))),
            top,
        )
    }

    /// Lines `0` to `n - 1`, with a one-character cursor at the start of line `cursor`.
    fn numbered(n: usize, cursor: usize) -> String {
        (0..n)
            .map(|i| {
                let s = i.to_string();
                if i == cursor {
                    format!("#[{}|]#{}\n", &s[..1], &s[1..])
                } else {
                    format!("{s}\n")
                }
            })
            .collect()
    }

    #[test]
    fn window_motions_respect_scrolloff() {
        // A 20-line view from line 10: gt goes 5 below the top, gb 5 above the bottom.
        let (sel, _) = in_view(&numbered(100, 50), 10, 20, goto_window_top);
        assert_eq!(sel, numbered(100, 15));
        let (sel, _) = in_view(&numbered(100, 50), 10, 20, goto_window_bottom);
        assert_eq!(sel, numbered(100, 24));
        let (sel, _) = in_view(&numbered(100, 50), 10, 20, goto_window_center);
        assert_eq!(sel, numbered(100, 19));
        // Near the end of the file the bottom is the last line, less scrolloff.
        let (sel, _) = in_view(&numbered(12, 0), 0, 20, goto_window_bottom);
        assert!(sel.contains("#[7|]#"), "{sel}");
    }

    #[test]
    fn paging() {
        let (sel, top) = in_view(&numbered(100, 0), 0, 20, page_down);
        assert_eq!(top, 20);
        assert_eq!(
            sel,
            numbered(100, 25),
            "the cursor lands scrolloff below the new top"
        );
        let (sel, top) = in_view(&numbered(100, 50), 40, 20, page_up);
        assert_eq!(top, 20);
        assert_eq!(sel, numbered(100, 34));
        let (sel, top) = in_view(&numbered(100, 0), 0, 20, page_cursor_half_down);
        assert_eq!((top, sel), (10, numbered(100, 10)));
        // Paging up from the top stays put.
        let (_, top) = in_view(&numbered(100, 3), 0, 20, page_up);
        assert_eq!(top, 0);
        // The view doesn't go past the last line: the empty one after the final newline.
        let (_, top) = in_view(&numbered(30, 0), 25, 20, page_down);
        assert_eq!(top, 30);
    }

    #[test]
    fn aligning_the_view() {
        let (_, top) = in_view(&numbered(100, 50), 0, 20, align_view_top);
        assert_eq!(top, 50);
        let (_, top) = in_view(&numbered(100, 50), 0, 20, align_view_bottom);
        assert_eq!(top, 31);
        let (_, top) = in_view(&numbered(100, 50), 0, 20, align_view_center);
        assert_eq!(top, 41);
        let (_, top) = in_view(&numbered(100, 2), 0, 20, align_view_center);
        assert_eq!(top, 0, "can't scroll above the first line");
    }

    #[test]
    fn edges_of_the_file_and_line() {
        // h at the start stays; l can step onto the end of a file with no final newline, as
        // in Helix.
        assert_eq!(run("#[a|]#b", &[&move_char_left]), "#[a|]#b");
        assert_eq!(run("a#[b|]#", &[&move_char_right]), "ab#[|]#");
        // x on the last line without a newline selects to the end.
        assert_eq!(
            run("a\n#[b|]#", &[&extend_line_below, &extend_line_below]),
            "a\n#[b|]#"
        );
        // gl stops before a CRLF line ending.
        assert_eq!(run("#[a|]#b\r\ncd", &[&goto_line_end]), "a#[b|]#\r\ncd");
        // gs on a blank line stays.
        assert_eq!(
            run("a\n#[ |]# \n", &[&goto_first_nonwhitespace]),
            "a\n#[ |]# \n"
        );
        // mm with no bracket under the cursor stays.
        assert_eq!(run("#[a|]#(b)", &[&match_brackets]), "#[a|]#(b)");
        // ge on a file without a trailing newline goes to its last line.
        assert_eq!(run("#[a|]#\nb", &[&goto_last_line]), "a\n#[b|]#");
        // A count past the end goes to the last line.
        assert_eq!(
            run("#[a|]#\nb\n", &[&counted(99, goto_line)]),
            "a\n#[b|]#\n"
        );
    }

    #[test]
    fn paragraph_text_object_and_change_counts() {
        assert_eq!(
            run(
                "a\n#[b|]#\n\nc\n",
                &[&|cx| select_textobject(cx, false, 'p')]
            ),
            "#[a\nb\n|]#\nc\n"
        );
        // Text objects that need tree-sitter aren't supported: the selection stays.
        assert_eq!(
            run("#[a|]#", &[&|cx| select_textobject(cx, false, 'f')]),
            "#[a|]#"
        );
        let hunks = [
            Hunk {
                before: 0..1,
                after: 0..1,
            },
            Hunk {
                before: 2..3,
                after: 2..3,
            },
            Hunk {
                before: 4..5,
                after: 4..5,
            },
        ];
        let t = "#[a|]#\nb\nc\nd\ne\n";
        assert_eq!(
            run_with(t, false, &hunks, &[&counted(2, goto_next_change)]),
            "a\nb\nc\nd\n#[e\n|]#"
        );
        assert_eq!(
            run_with(t, false, &hunks, &[&goto_first_change]),
            "#[a\n|]#b\nc\nd\ne\n"
        );
        // No hunks: a message, and the cursor stays.
        let (text, sel) = print("#[a|]#\n");
        let rope = Rope::from(text);
        let mut cx = Ctx {
            text: &rope,
            sel,
            count: None,
            extend: false,
            hunks: &[],
            top: 0,
            left: 0,
            height: 10,
            fmt: TextFormat::default(),
            status: None,
            center: false,
            keep_top: false,
        };
        goto_next_change(&mut cx);
        assert_eq!(
            cx.status.as_ref().map(|s| s.0.as_str()),
            Some("No changes in this file")
        );
        goto_first_change(&mut cx);
        assert_eq!(plain(rope.clone(), &cx.sel), "#[a|]#\n");
    }

    #[test]
    fn select_mode_extends_every_motion_kind() {
        let e = |input, f: &dyn Fn(&mut Ctx)| run_with(input, true, &[], &[f]);
        assert_eq!(e("#[a|]#bc\nd", &goto_line_end), "#[abc|]#\nd");
        assert_eq!(e("ab#[c|]#", &goto_line_start), "#[|abc]#");
        assert_eq!(e("#[a|]#\nb\n", &goto_last_line), "#[a\nb|]#\n");
        assert_eq!(e("#[a|]#bcb", &|cx| find_next_char(cx, 'b')), "#[ab|]#cb");
        let hunks = [Hunk {
            before: 2..2,
            after: 2..3,
        }];
        assert_eq!(
            run_with("#[a|]#\nb\nc\n", true, &hunks, &[&goto_next_change]),
            "#[a\nb\nc\n|]#"
        );
    }
}
