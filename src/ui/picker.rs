//! Helix's picker: a fuzzy-filtered list (nucleo, Helix's matcher) on the left and a preview on
//! the right, in an overlay of 90% of the screen.

use std::borrow::Cow;

use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
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

use super::{
    comments::Threads,
    component::{Component, Cx, Msg, Outcome},
    keymap::{Action, Context, Key, Keymaps, Node},
    view::{self, clean},
};
use crate::theme::Theme;

/// Below this width Helix drops the preview.
const MIN_WIDTH_FOR_PREVIEW: u16 = 72;

/// What a picker's keys do. `keymap.rs` binds them.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PickerCmd {
    Next,
    Prev,
    PageDown,
    PageUp,
    First,
    Last,
    Choose,
    Close,
    DeleteChar,
}

/// What choosing an item opens.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Change(usize),
    Path(String),
    /// A thread, by its id.
    Thread(String),
    /// A command, by its name: the palette runs it.
    Command(&'static str),
    /// A review, by its id: the dashboard opens it.
    Review(String),
    /// Nothing to open: a key the palette only lists.
    Nothing,
}

/// The picker's keys, as `C-g` lists them.
/// `C-g` over a picker in `glos dashboard`: the picker's keys from `keymaps`, in a picker.
pub fn keys_palette(theme: &Theme, keymaps: &Keymaps) -> Picker {
    let style = theme.get("ui.text.focus");
    let mut rows = vec![("type".to_string(), "Filter the list")];
    rows.extend(
        keymaps
            .get(Context::Picker)
            .help()
            .into_iter()
            .map(|(k, d, _)| (k, d)),
    );
    let items = rows
        .into_iter()
        .map(|(k, d)| Item {
            tags: vec![(k, style)],
            text: d.to_string(),
            target: Target::Nothing,
        })
        .collect();
    let mut p = Picker::new(vec!["keys", Context::Picker.title()], items);
    p.title = Some(Context::Picker.border_title());
    p.preview = false;
    p.palette = true;
    p
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
    /// A preview beside the list, when there's room; the palette has none.
    pub preview: bool,
    /// The keys palette (`C-g`): it opens over another picker, and closing it goes back there.
    pub palette: bool,
    /// Drawn in the top border, as the palette names its keymap context.
    pub title: Option<String>,
    /// How many lines the wheel has scrolled the preview, and which item it was showing. When
    /// the selection changes, the scroll resets so the new preview starts centred.
    preview_scroll: (Option<Target>, isize),
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
            preview: true,
            palette: false,
            title: None,
            preview_scroll: (None, 0),
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
                    let s =
                        pattern.score(Utf32Str::new(&item.text, &mut buf), &mut self.matcher)?;
                    Some((i as u32, s))
                })
                .collect();
            // As nucleo's worker sorts them: best score, then the shorter text, then list order.
            let len = |i: u32| self.items[i as usize].text.chars().count();
            scored.sort_by(|a, b| {
                b.1.cmp(&a.1)
                    .then(len(a.0).cmp(&len(b.0)))
                    .then(a.0.cmp(&b.0))
            });
            self.matched = scored.into_iter().map(|(i, _)| i).collect();
        }
        self.cursor = self.cursor.min(self.matched.len().saturating_sub(1));
    }

    /// New items, as when what they list changed; the query stays and the cursor stays put.
    pub fn set_items(&mut self, items: Vec<Item>) {
        self.items = items;
        self.refilter();
    }

    pub fn selected(&self) -> Option<&Target> {
        self.matched
            .get(self.cursor)
            .map(|i| &self.items[*i as usize].target)
    }

    /// Helix's picker keys; `page` is how far a page key moves.
    /// Runs one of the picker's commands. `page` is how many rows a page moves.
    fn run(&mut self, cmd: PickerCmd, page: usize) -> Outcome {
        let n = self.matched.len().max(1);
        let last = self.matched.len().saturating_sub(1);
        match cmd {
            PickerCmd::Close => return Outcome::Msg(Msg::Close),
            PickerCmd::Choose => {
                return Outcome::Msg(self.selected().cloned().map_or(Msg::Close, Msg::Open));
            }
            PickerCmd::Prev => self.cursor = (self.cursor + n - 1) % n,
            PickerCmd::Next => self.cursor = (self.cursor + 1) % n,
            PickerCmd::PageDown => self.cursor = (self.cursor + page).min(last),
            PickerCmd::PageUp => self.cursor = self.cursor.saturating_sub(page),
            PickerCmd::First => self.cursor = 0,
            PickerCmd::Last => self.cursor = last,
            PickerCmd::DeleteChar => {
                self.query.pop();
                self.refilter();
            }
        }
        Outcome::Consumed
    }

    /// A character typed into the filter.
    fn type_char(&mut self, c: char) -> Outcome {
        self.query.push(c);
        self.cursor = 0;
        self.refilter();
        Outcome::Consumed
    }

    /// A key, through the picker's keys in `keymaps`: a command, or a character for the
    /// filter. A command for anything other than the picker is `Ignored`, for the caller.
    pub fn press(&mut self, keymaps: &Keymaps, key: Key, cx: &Cx) -> Outcome {
        let page = self.page(self.areas(cx.screen).0);
        self.press_paged(keymaps, key, page)
    }

    fn press_paged(&mut self, keymaps: &Keymaps, key: Key, page: usize) -> Outcome {
        match keymaps.get(Context::Picker).get(key) {
            Some(Node::Cmd(c)) => match c.action {
                Action::Picker(cmd) => self.run(cmd, page),
                _ => Outcome::Ignored,
            },
            Some(Node::Menu(_)) => Outcome::Consumed,
            None => match key.char() {
                Some(c) => self.type_char(c),
                None => Outcome::Consumed,
            },
        }
    }

    #[cfg(test)]
    fn key_paged(&mut self, key: Key, page: usize) -> Outcome {
        self.press_paged(&Keymaps::default(), key, page)
    }

    /// Where the picker and its preview go: 90% of the screen above the bottom two rows.
    pub fn areas(&self, screen: Rect) -> (Rect, Option<Rect>) {
        let avail = Rect {
            height: screen.height.saturating_sub(2),
            ..screen
        };
        let (w, h) = (avail.width * 90 / 100, avail.height * 90 / 100);
        let area = Rect::new(
            avail.x + (avail.width - w) / 2,
            avail.y + (avail.height - h) / 2,
            w,
            h,
        );
        if self.preview && area.width > MIN_WIDTH_FOR_PREVIEW {
            let half = area.width / 2;
            (
                Rect {
                    width: half,
                    ..area
                },
                Some(Rect {
                    x: area.x + half,
                    width: area.width - half,
                    ..area
                }),
            )
        } else {
            (area, None)
        }
    }

    /// The item drawn at a screen cell, for mouse clicks.
    pub fn target_at(&self, area: Rect, x: u16, y: u16) -> Option<Target> {
        let header = (self.columns.len() > 1) as u16;
        let first = area.y + 1 + 2 + header;
        if x <= area.x || x + 1 >= area.right() || y < first || y + 1 >= area.bottom() {
            return None;
        }
        let rows = self.page(area);
        let offset = self.cursor - self.cursor % rows;
        let i = *self.matched.get(offset + (y - first) as usize)?;
        Some(self.items[i as usize].target.clone())
    }

    /// Rows of items that fit, for paging.
    pub fn page(&self, area: Rect) -> usize {
        let header = (self.columns.len() > 1) as u16;
        area.height.saturating_sub(2 + 2 + header).max(1) as usize
    }

    fn render_list(&mut self, buf: &mut Buffer, area: Rect, theme: &Theme) {
        let text_style = theme.get("ui.text");
        let selected = theme.get("ui.text.focus");
        let highlight = theme.get("special").add_modifier(Modifier::BOLD);
        buf.set_style(area, theme.get("ui.background"));
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                buf[(x, y)].set_symbol(" ");
            }
        }
        let mut block = Block::default().borders(Borders::ALL);
        if let Some(t) = &self.title {
            block = block.title(format!(" {} ", clean(t)));
        }
        let inner = block.inner(area);
        block.render(area, buf);
        if inner.height < 3 {
            return;
        }

        // The prompt, with the match count on the right.
        let count = format!("{}/{}", self.matched.len(), self.items.len());
        let line = Rect {
            x: inner.x + 1,
            width: inner.width.saturating_sub(1),
            height: 1,
            ..inner
        };
        let room = line.width.saturating_sub(count.len() as u16 + 1) as usize;
        buf.set_stringn(line.x, line.y, clean(&self.query), room, text_style);
        let cx = line.x + self.query.width().min(room) as u16;
        if cx < line.right() {
            buf[(cx, line.y)].set_style(text_style.add_modifier(Modifier::REVERSED));
        }
        buf.set_stringn(
            line.right().saturating_sub(count.len() as u16 + 1),
            line.y,
            &count,
            count.len(),
            text_style,
        );
        let sep = theme.get("ui.background.separator");
        for x in inner.left()..inner.right() {
            buf[(x, inner.y + 1)].set_symbol("─").set_style(sep);
        }

        let mut y = inner.y + 2;
        let rows = self.page(area);
        let offset = self.cursor - self.cursor % rows;
        let visible: Vec<u32> = self
            .matched
            .iter()
            .skip(offset)
            .take(rows)
            .copied()
            .collect();
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
                buf.set_stringn(
                    x,
                    y,
                    *h,
                    inner.right().saturating_sub(x) as usize,
                    col_style,
                );
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
            let row_style = if is_sel {
                text_style.patch(selected)
            } else {
                text_style
            };
            let right = inner.right();
            buf.set_stringn(inner.x, y, if is_sel { " > " } else { "   " }, 3, row_style);
            let mut x = inner.x + 3;
            for (c, (t, s)) in item.tags.iter().enumerate().take(ntags) {
                buf.set_stringn(
                    x,
                    y,
                    clean(t),
                    right.saturating_sub(x) as usize,
                    row_style.patch(*s),
                );
                x += widths[c] as u16 + 1;
            }
            // The filtered column, with matched characters highlighted as Helix does.
            indices.clear();
            if !self.query.is_empty() {
                pattern.indices(
                    Utf32Str::new(&item.text, &mut buf32),
                    &mut self.matcher,
                    &mut indices,
                );
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
                let style = if hit {
                    row_style.patch(highlight)
                } else {
                    row_style
                };
                buf[(x, y)].set_symbol(&s).set_style(style);
                x += s.width().max(1) as u16;
            }
            y += 1;
        }
    }
}

impl Component for Picker {
    type Cmd = PickerCmd;

    fn command(&mut self, cmd: PickerCmd, cx: &Cx) -> Outcome {
        let page = self.page(self.areas(cx.screen).0);
        self.run(cmd, page)
    }

    fn text(&mut self, c: char, _cx: &Cx) -> Outcome {
        self.type_char(c)
    }

    /// The wheel moves; a click opens the item under it. The picker places itself on the
    /// screen, so `area` isn't used.
    fn mouse(&mut self, m: MouseEvent, _area: Rect, cx: &Cx) -> Outcome {
        let (_, preview) = self.areas(cx.screen);
        if let Some(p) = preview
            && p.contains((m.column, m.row).into())
        {
            let by = match m.kind {
                MouseEventKind::ScrollDown => 3,
                MouseEventKind::ScrollUp => -3,
                _ => return Outcome::Consumed,
            };
            self.preview_scroll.1 += by;
            return Outcome::Consumed;
        }
        match m.kind {
            MouseEventKind::ScrollDown => self.run(PickerCmd::Next, 1),
            MouseEventKind::ScrollUp => self.run(PickerCmd::Prev, 1),
            MouseEventKind::Down(MouseButton::Left) => {
                let (list, _) = self.areas(cx.screen);
                match self.target_at(list, m.column, m.row) {
                    Some(t) => Outcome::Msg(Msg::Open(t)),
                    None => Outcome::Consumed,
                }
            }
            _ => Outcome::Consumed,
        }
    }

    /// The list over most of the screen (`area`), and beside it, when there's room, the selected
    /// file: centred on the thread, or on its first change, as Helix centres a preview.
    fn render(&mut self, buf: &mut Buffer, area: Rect, cx: &Cx) {
        let (list, preview) = self.areas(area);
        self.render_list(buf, list, cx.theme);
        let Some(preview) = preview else { return };
        let inner = preview_frame(buf, preview, cx.theme);
        let Some(state) = cx.state else { return };
        if self.preview_scroll.0.as_ref() != self.selected() {
            self.preview_scroll = (self.selected().cloned(), 0);
        }
        let Some((file, line)) = self.selected().and_then(|t| state.preview(t)) else {
            return;
        };
        let line = line.or(file.hunks.first().map(|h| h.after.start as usize));
        let centred = line.map_or(0, |l| l.saturating_sub(inner.height as usize / 3)) as isize;
        let off = self.preview_scroll.1;
        let last = file.text.len_lines().saturating_sub(1) as isize;
        let top = (centred + off).clamp(0, last.max(0));
        // Don't let the offset grow past either end of the file. Otherwise, after scrolling
        // too far, it would take several turns of the wheel before the preview moved back.
        self.preview_scroll.1 = top - centred;
        let top = top as usize;
        view::draw(
            buf,
            inner,
            file,
            top,
            0,
            state.wrap,
            &state.loader,
            cx.theme,
            None,
            &Threads::default(),
        );
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
    Rect {
        x: inner.x + 1,
        width: inner.width.saturating_sub(2),
        ..inner
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyModifiers};

    fn picker(paths: &[&str]) -> Picker {
        let items = paths
            .iter()
            .map(|p| Item {
                tags: vec![],
                text: p.to_string(),
                target: Target::Path(p.to_string()),
            })
            .collect();
        Picker::new(vec!["path"], items)
    }

    fn typed(p: &mut Picker, s: &str) {
        for c in s.chars() {
            p.key_paged(
                Key {
                    code: KeyCode::Char(c),
                    mods: KeyModifiers::NONE,
                },
                10,
            );
        }
    }

    #[test]
    fn filters_and_moves() {
        let mut p = picker(&["src/main.rs", "src/ui/mod.rs", "README.md"]);
        assert_eq!(p.selected(), Some(&Target::Path("src/main.rs".into())));
        typed(&mut p, "uimod");
        assert_eq!(p.matched.len(), 1);
        assert_eq!(p.selected(), Some(&Target::Path("src/ui/mod.rs".into())));
        p.key_paged(
            Key {
                code: KeyCode::Backspace,
                mods: KeyModifiers::NONE,
            },
            10,
        );
        p.key_paged(
            Key {
                code: KeyCode::Backspace,
                mods: KeyModifiers::NONE,
            },
            10,
        );
        p.key_paged(
            Key {
                code: KeyCode::Backspace,
                mods: KeyModifiers::NONE,
            },
            10,
        );
        p.key_paged(
            Key {
                code: KeyCode::Backspace,
                mods: KeyModifiers::NONE,
            },
            10,
        );
        p.key_paged(
            Key {
                code: KeyCode::Backspace,
                mods: KeyModifiers::NONE,
            },
            10,
        );
        assert_eq!(p.matched.len(), 3);
        p.key_paged(
            Key {
                code: KeyCode::Up,
                mods: KeyModifiers::NONE,
            },
            10,
        );
        assert_eq!(p.selected(), Some(&Target::Path("README.md".into())));
        assert!(matches!(
            p.key_paged(
                Key {
                    code: KeyCode::Esc,
                    mods: KeyModifiers::NONE
                },
                10
            ),
            Outcome::Msg(Msg::Close)
        ));
    }

    #[test]
    fn keys_wrap_page_and_jump() {
        let names: Vec<String> = (0..30).map(|i| format!("f{i:02}")).collect();
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let mut p = picker(&refs);
        let key = |code| Key {
            code,
            mods: KeyModifiers::NONE,
        };
        let ctrl = |c| Key {
            code: KeyCode::Char(c),
            mods: KeyModifiers::CONTROL,
        };
        let at = |p: &Picker| p.selected().cloned();
        p.key_paged(key(KeyCode::Up), 10);
        assert_eq!(
            at(&p),
            Some(Target::Path("f29".into())),
            "up wraps to the end"
        );
        p.key_paged(key(KeyCode::Down), 10);
        assert_eq!(
            at(&p),
            Some(Target::Path("f00".into())),
            "down wraps to the start"
        );
        p.key_paged(ctrl('n'), 10);
        p.key_paged(key(KeyCode::Tab), 10);
        assert_eq!(at(&p), Some(Target::Path("f02".into())));
        p.key_paged(ctrl('p'), 10);
        p.key_paged(Key::parse("S-tab").unwrap(), 10);
        assert_eq!(at(&p), Some(Target::Path("f00".into())));
        p.key_paged(ctrl('d'), 10);
        assert_eq!(at(&p), Some(Target::Path("f10".into())));
        p.key_paged(key(KeyCode::PageDown), 100);
        assert_eq!(
            at(&p),
            Some(Target::Path("f29".into())),
            "pages stop at the end"
        );
        p.key_paged(ctrl('u'), 10);
        assert_eq!(at(&p), Some(Target::Path("f19".into())));
        p.key_paged(key(KeyCode::Home), 10);
        assert_eq!(at(&p), Some(Target::Path("f00".into())));
        p.key_paged(key(KeyCode::End), 10);
        assert_eq!(at(&p), Some(Target::Path("f29".into())));
        assert!(matches!(
            p.key_paged(ctrl('c'), 10),
            Outcome::Msg(Msg::Close)
        ));
    }

    #[test]
    fn no_matches_and_enter() {
        let mut p = picker(&["a.rs", "b.rs"]);
        typed(&mut p, "zzz");
        assert_eq!(p.selected(), None);
        assert!(matches!(
            p.key_paged(
                Key {
                    code: KeyCode::Enter,
                    mods: KeyModifiers::NONE
                },
                10
            ),
            Outcome::Msg(Msg::Close)
        ));
        let mut p = picker(&["a.rs", "b.rs"]);
        typed(&mut p, "b");
        assert!(matches!(
            p.key_paged(Key { code: KeyCode::Enter, mods: KeyModifiers::NONE }, 10),
            Outcome::Msg(Msg::Open(Target::Path(ref s))) if s == "b.rs"
        ));
        // Alt and ctrl characters aren't typed into the query.
        let mut p = picker(&["a.rs"]);
        p.key_paged(
            Key {
                code: KeyCode::Char('x'),
                mods: KeyModifiers::ALT,
            },
            10,
        );
        assert_eq!(p.query, "");
    }

    #[test]
    fn best_match_first_and_smart_case() {
        let mut p = picker(&["src/ui/view.rs", "docs/overview.md", "view.rs"]);
        typed(&mut p, "view.rs");
        assert_eq!(p.selected(), Some(&Target::Path("view.rs".into())));
        let mut p = picker(&["README.md", "readme.txt"]);
        typed(&mut p, "READ");
        assert_eq!(p.matched.len(), 1, "an uppercase query matches case");
    }

    #[test]
    fn clicks_map_to_rows() {
        let p = picker(&["a", "b", "c"]);
        let area = Rect::new(0, 0, 40, 10);
        // Border, prompt, separator, then the items.
        assert_eq!(p.target_at(area, 5, 3), Some(Target::Path("a".into())));
        assert_eq!(p.target_at(area, 5, 5), Some(Target::Path("c".into())));
        assert_eq!(p.target_at(area, 5, 6), None);
        assert_eq!(p.target_at(area, 0, 3), None, "the border");
        assert_eq!(p.target_at(area, 5, 1), None, "the prompt");
    }

    #[test]
    fn as_a_component() {
        use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
        let mut p = picker(&["a", "b", "c"]);
        let theme =
            crate::theme::Theme::load("default", crate::theme::ColorSupport::TrueColor).unwrap();
        let cx = Cx {
            screen: Rect::new(0, 0, 50, 14),
            theme: &theme,
            state: None,
        };
        let mouse = |kind, column, row| MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        };
        assert_eq!(p.command(PickerCmd::Next, &cx), Outcome::Consumed);
        assert_eq!(p.selected(), Some(&Target::Path("b".into())));
        let wheel = mouse(MouseEventKind::ScrollDown, 10, 5);
        assert_eq!(p.mouse(wheel, cx.screen, &cx), Outcome::Consumed);
        assert_eq!(p.selected(), Some(&Target::Path("c".into())));
        // The list is 90% of the screen above the bottom two rows: row 1 is its top border.
        let (list, _) = p.areas(cx.screen);
        let click = mouse(MouseEventKind::Down(MouseButton::Left), 5, list.y + 3);
        assert_eq!(
            p.mouse(click, cx.screen, &cx),
            Outcome::Msg(Msg::Open(Target::Path("a".into())))
        );
        let off = mouse(MouseEventKind::Down(MouseButton::Left), 0, 0);
        assert_eq!(p.mouse(off, cx.screen, &cx), Outcome::Consumed);
        assert_eq!(p.command(PickerCmd::Close, &cx), Outcome::Msg(Msg::Close));
    }
}
