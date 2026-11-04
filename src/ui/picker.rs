//! Helix's picker: a fuzzy-filtered list (nucleo, Helix's matcher) on the left and a preview on
//! the right, in an overlay of 90% of the screen.

use std::borrow::Cow;

use crossterm::event::{KeyCode, KeyModifiers};
use nucleo::{
    Matcher, Utf32Str,
    pattern::{CaseMatching, Normalization, Pattern},
};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
    widgets::{Block, Borders, Widget},
};
use unicode_width::UnicodeWidthStr;

use super::{keymap::Key, view::clean};
use crate::theme::Theme;

/// Below this width Helix drops the preview.
const MIN_WIDTH_FOR_PREVIEW: u16 = 72;

/// What choosing an item opens.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Change(usize),
    Path(String),
}

pub struct Item {
    /// Every column but the last, already styled (e.g. `~ modified`).
    pub tags: Vec<(String, Style)>,
    /// The last column, which the query filters.
    pub text: String,
    pub target: Target,
}

pub struct Picker {
    /// Column headers; shown only when there's more than one column, as in Helix.
    pub columns: Vec<&'static str>,
    items: Vec<Item>,
    query: String,
    /// Indices into `items`, best match first.
    matched: Vec<u32>,
    cursor: usize,
    matcher: Matcher,
}

pub enum Outcome {
    Stay,
    Close,
    Open(Target),
}

impl Picker {
    pub fn new(columns: Vec<&'static str>, items: Vec<Item>) -> Picker {
        let mut p = Picker {
            columns,
            matched: (0..items.len() as u32).collect(),
            items,
            query: String::new(),
            cursor: 0,
            matcher: Matcher::new(nucleo::Config::DEFAULT.match_paths()),
        };
        p.refilter();
        p
    }

    fn refilter(&mut self) {
        if self.query.is_empty() {
            self.matched = (0..self.items.len() as u32).collect();
        } else {
            let pattern = Pattern::parse(&self.query, CaseMatching::Smart, Normalization::Smart);
            let mut buf = vec![];
            let mut scored: Vec<(u32, u32)> = self
                .items
                .iter()
                .enumerate()
                .filter_map(|(i, item)| {
                    let s = pattern.score(Utf32Str::new(&item.text, &mut buf), &mut self.matcher)?;
                    Some((i as u32, s))
                })
                .collect();
            scored.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
            self.matched = scored.into_iter().map(|(i, _)| i).collect();
        }
        self.cursor = self.cursor.min(self.matched.len().saturating_sub(1));
    }

    pub fn selected(&self) -> Option<&Target> {
        self.matched.get(self.cursor).map(|i| &self.items[*i as usize].target)
    }

    /// Helix's picker keys.
    pub fn key(&mut self, key: Key, page: usize) -> Outcome {
        let ctrl = key.mods.contains(KeyModifiers::CONTROL);
        let n = self.matched.len();
        match key.code {
            KeyCode::Esc => return Outcome::Close,
            KeyCode::Char('c') if ctrl => return Outcome::Close,
            KeyCode::Enter => return self.selected().cloned().map_or(Outcome::Close, Outcome::Open),
            KeyCode::BackTab | KeyCode::Up => self.cursor = (self.cursor + n.max(1) - 1) % n.max(1),
            KeyCode::Char('p') if ctrl => self.cursor = (self.cursor + n.max(1) - 1) % n.max(1),
            KeyCode::Tab | KeyCode::Down => self.cursor = (self.cursor + 1) % n.max(1),
            KeyCode::Char('n') if ctrl => self.cursor = (self.cursor + 1) % n.max(1),
            KeyCode::PageDown => self.cursor = (self.cursor + page).min(n.saturating_sub(1)),
            KeyCode::Char('d') if ctrl => self.cursor = (self.cursor + page).min(n.saturating_sub(1)),
            KeyCode::PageUp => self.cursor = self.cursor.saturating_sub(page),
            KeyCode::Char('u') if ctrl => self.cursor = self.cursor.saturating_sub(page),
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = n.saturating_sub(1),
            KeyCode::Backspace => {
                self.query.pop();
                self.refilter();
            }
            KeyCode::Char(c) if !ctrl && !key.mods.contains(KeyModifiers::ALT) => {
                self.query.push(c);
                self.cursor = 0;
                self.refilter();
            }
            _ => {}
        }
        Outcome::Stay
    }

    /// Where the picker and its preview go: 90% of the screen above the bottom two rows.
    pub fn areas(screen: Rect) -> (Rect, Option<Rect>) {
        let avail = Rect { height: screen.height.saturating_sub(2), ..screen };
        let (w, h) = (avail.width * 90 / 100, avail.height * 90 / 100);
        let area = Rect::new(avail.x + (avail.width - w) / 2, avail.y + (avail.height - h) / 2, w, h);
        if area.width > MIN_WIDTH_FOR_PREVIEW {
            let half = area.width / 2;
            (Rect { width: half, ..area }, Some(Rect { x: area.x + half, width: area.width - half, ..area }))
        } else {
            (area, None)
        }
    }

    /// Rows of items that fit, for paging.
    pub fn page(&self, area: Rect) -> usize {
        let header = (self.columns.len() > 1) as u16;
        area.height.saturating_sub(2 + 2 + header).max(1) as usize
    }

    pub fn render(&mut self, buf: &mut Buffer, area: Rect, theme: &Theme) {
        let text_style = theme.get("ui.text");
        let selected = theme.get("ui.text.focus");
        let highlight = theme.get("special").add_modifier(Modifier::BOLD);
        buf.set_style(area, theme.get("ui.background"));
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                buf[(x, y)].set_symbol(" ");
            }
        }
        let block = Block::default().borders(Borders::ALL);
        let inner = block.inner(area);
        block.render(area, buf);
        if inner.height < 3 {
            return;
        }

        // The prompt, with the match count on the right.
        let count = format!("{}/{}", self.matched.len(), self.items.len());
        let line = Rect { x: inner.x + 1, width: inner.width.saturating_sub(1), height: 1, ..inner };
        let room = line.width.saturating_sub(count.len() as u16 + 1) as usize;
        buf.set_stringn(line.x, line.y, clean(&self.query), room, text_style);
        let cx = line.x + self.query.width().min(room) as u16;
        if cx < line.right() {
            buf[(cx, line.y)].set_style(text_style.add_modifier(Modifier::REVERSED));
        }
        buf.set_stringn(line.right().saturating_sub(count.len() as u16 + 1), line.y, &count, count.len(), text_style);
        let sep = theme.get("ui.background.separator");
        for x in inner.left()..inner.right() {
            buf[(x, inner.y + 1)].set_symbol("─").set_style(sep);
        }

        let mut y = inner.y + 2;
        let rows = self.page(area);
        let offset = self.cursor - self.cursor % rows;
        let visible: Vec<u32> = self.matched.iter().skip(offset).take(rows).copied().collect();
        // Column widths: the widest tag in each column among the visible rows.
        let ntags = self.columns.len().saturating_sub(1);
        let mut widths = vec![0usize; ntags];
        for i in &visible {
            for (c, (t, _)) in self.items[*i as usize].tags.iter().enumerate().take(ntags) {
                widths[c] = widths[c].max(t.width());
            }
        }
        for (c, h) in self.columns.iter().enumerate().take(ntags) {
            widths[c] = widths[c].max(h.width());
        }
        if self.columns.len() > 1 {
            let header = theme.get("ui.picker.header");
            let col_style = header.patch(theme.get("ui.picker.header.column"));
            let mut x = inner.x + 3;
            for (c, h) in self.columns.iter().enumerate() {
                buf.set_stringn(x, y, *h, inner.right().saturating_sub(x) as usize, col_style);
                x += widths.get(c).copied().unwrap_or(0) as u16 + 1;
            }
            y += 1;
        }

        let pattern = Pattern::parse(&self.query, CaseMatching::Smart, Normalization::Smart);
        let mut buf32 = vec![];
        let mut indices = vec![];
        for (row, i) in visible.iter().enumerate() {
            if y >= inner.bottom() {
                break;
            }
            let item = &self.items[*i as usize];
            let is_sel = offset + row == self.cursor;
            let row_style = if is_sel { text_style.patch(selected) } else { text_style };
            let right = inner.right();
            buf.set_stringn(inner.x, y, if is_sel { " > " } else { "   " }, 3, row_style);
            let mut x = inner.x + 3;
            for (c, (t, s)) in item.tags.iter().enumerate().take(ntags) {
                buf.set_stringn(x, y, clean(t), right.saturating_sub(x) as usize, row_style.patch(*s));
                x += widths[c] as u16 + 1;
            }
            // The filtered column, with matched characters highlighted as Helix does.
            indices.clear();
            if !self.query.is_empty() {
                pattern.indices(Utf32Str::new(&item.text, &mut buf32), &mut self.matcher, &mut indices);
                indices.sort_unstable();
            }
            let mut next = indices.iter().peekable();
            for (gi, ch) in item.text.chars().enumerate() {
                if x >= right {
                    break;
                }
                let hit = next.peek().is_some_and(|n| **n == gi as u32);
                if hit {
                    next.next();
                }
                let s: Cow<str> = clean(&ch.to_string()).into_owned().into();
                let style = if hit { row_style.patch(highlight) } else { row_style };
                buf[(x, y)].set_symbol(&s).set_style(style);
                x += s.width().max(1) as u16;
            }
            y += 1;
        }
    }
}

/// The bordered frame of the preview, and the area inside it for the text.
pub fn preview_frame(buf: &mut Buffer, area: Rect, theme: &Theme) -> Rect {
    buf.set_style(area, theme.get("ui.background"));
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            buf[(x, y)].set_symbol(" ");
        }
    }
    let block = Block::default().borders(Borders::ALL);
    let inner = block.inner(area);
    block.render(area, buf);
    Rect { x: inner.x + 1, width: inner.width.saturating_sub(2), ..inner }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn picker(paths: &[&str]) -> Picker {
        let items = paths
            .iter()
            .map(|p| Item { tags: vec![], text: p.to_string(), target: Target::Path(p.to_string()) })
            .collect();
        Picker::new(vec!["path"], items)
    }

    fn typed(p: &mut Picker, s: &str) {
        for c in s.chars() {
            p.key(Key { code: KeyCode::Char(c), mods: KeyModifiers::NONE }, 10);
        }
    }

    #[test]
    fn filters_and_moves() {
        let mut p = picker(&["src/main.rs", "src/ui/mod.rs", "README.md"]);
        assert_eq!(p.selected(), Some(&Target::Path("src/main.rs".into())));
        typed(&mut p, "uimod");
        assert_eq!(p.matched.len(), 1);
        assert_eq!(p.selected(), Some(&Target::Path("src/ui/mod.rs".into())));
        p.key(Key { code: KeyCode::Backspace, mods: KeyModifiers::NONE }, 10);
        p.key(Key { code: KeyCode::Backspace, mods: KeyModifiers::NONE }, 10);
        p.key(Key { code: KeyCode::Backspace, mods: KeyModifiers::NONE }, 10);
        p.key(Key { code: KeyCode::Backspace, mods: KeyModifiers::NONE }, 10);
        p.key(Key { code: KeyCode::Backspace, mods: KeyModifiers::NONE }, 10);
        assert_eq!(p.matched.len(), 3);
        p.key(Key { code: KeyCode::Up, mods: KeyModifiers::NONE }, 10);
        assert_eq!(p.selected(), Some(&Target::Path("README.md".into())));
        assert!(matches!(p.key(Key { code: KeyCode::Esc, mods: KeyModifiers::NONE }, 10), Outcome::Close));
    }
}
