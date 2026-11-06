//! Tests for the mouse. A click moves focus to what it lands on, and the wheel scrolls
//! whatever is under the pointer.

use super::*;

const CLICK: MouseEventKind = MouseEventKind::Down(MouseButton::Left);
const DOWN: MouseEventKind = MouseEventKind::ScrollDown;
const UP: MouseEventKind = MouseEventKind::ScrollUp;

/// Opens a review of 30 files, each 40 lines long, with the tree open and the code focused.
/// Every line is changed, so no unchanged code gets folded away. On the 72×14 screen, the tree
/// is 24 columns wide and shows 11 rows.
fn many() -> T {
    let mut t = T::folders(|o, n| {
        for i in 0..30 {
            std::fs::write(o.join(format!("f{i:02}.rs")), lines(40)).unwrap();
            let new: String = (1..=40).map(|l| format!("new {l}\n")).collect();
            std::fs::write(n.join(format!("f{i:02}.rs")), new).unwrap();
        }
    });
    t.keys("space e esc");
    assert_eq!(t.app.tree_area.width, 24);
    t
}

impl T {
    fn tree(&self) -> &tree::Tree {
        self.app.tree.as_ref().unwrap()
    }

    fn line(&self) -> usize {
        let f = self.app.main_file();
        f.sel.primary().cursor_line(f.text.slice(..))
    }

    fn mouse_with(&mut self, kind: MouseEventKind, column: u16, row: u16, mods: KeyModifiers) {
        self.app.mouse(MouseEvent {
            kind,
            column,
            row,
            modifiers: mods,
        });
        self.draw();
    }

    fn row(&self, y: usize) -> String {
        self.screen().lines().nth(y).unwrap().to_string()
    }
}

#[test]
fn clicks_focus_the_tree_or_the_code() {
    let mut t = many();
    assert!(!t.tree().focused);
    // Clicking the tree's title focuses the tree, so `j` moves the tree's cursor.
    t.mouse(CLICK, 5, 0);
    assert!(t.tree().focused);
    let line = t.line();
    t.keys("j");
    assert_eq!((t.tree().selected(), t.line()), (Some("f01.rs"), line));
    // Clicking the code moves focus back to it. Now `j` moves the code's cursor.
    let gutter = t.app.file().gutter_width() as u16;
    t.mouse(CLICK, 24 + gutter + 1, 3);
    assert!(!t.tree().focused);
    assert_eq!(t.line(), 3);
    t.keys("j");
    assert_eq!((t.tree().selected(), t.line()), (Some("f01.rs"), 4));
}

#[test]
fn a_click_on_a_tree_file_opens_it_and_keeps_focus() {
    let mut t = many();
    let y = t
        .screen()
        .lines()
        .position(|l| l.contains("f03.rs"))
        .unwrap() as u16;
    t.mouse(CLICK, 3, y);
    assert_eq!(t.path(), "f03.rs");
    assert!(t.tree().focused);
    t.keys("j");
    assert_eq!(
        (t.tree().selected(), t.path()),
        (Some("f04.rs"), "f03.rs".into())
    );
    // While the command line or the comment box is open, clicks in the tree are ignored.
    t.keys("esc :");
    t.mouse(CLICK, 3, y);
    assert_eq!(t.path(), "f03.rs");
    t.keys("esc x space c");
    t.mouse(CLICK, 3, y + 1);
    assert_eq!(t.path(), "f03.rs");
    assert!(t.app.comment_box.is_some());
}

#[test]
fn the_wheel_scrolls_what_it_is_over() {
    let mut t = many();
    // The wheel over the tree scrolls its rows. The tree's cursor doesn't move.
    t.mouse(DOWN, 3, 5).mouse(DOWN, 3, 5);
    assert_eq!((t.tree().top, t.tree().selected()), (6, Some("f00.rs")));
    assert!(t.row(1).starts_with(" f06.rs"), "{}", t.row(1));
    assert_eq!(t.top(), 0, "the code stays");
    // The wheel over the code scrolls the code, even while the tree has focus.
    t.mouse(CLICK, 3, 0);
    t.mouse(DOWN, 40, 5);
    assert_eq!((t.top(), t.tree().top), (3, 6));
    assert!(t.tree().focused, "the wheel doesn't move focus");
    // Moving the tree's cursor scrolls the tree back to show it.
    t.keys("j");
    assert_eq!(t.tree().top, 1);
    // The wheel over the statusline does nothing.
    t.mouse(DOWN, 40, 12);
    assert_eq!(t.top(), 3);
}

#[test]
fn the_wheel_scrolls_the_comment_box_and_the_code_under_it() {
    let mut t = T::files_sized("", &lines(100), 72, 14);
    t.keys("x space c");
    for i in 0..20 {
        t.typed(&format!("line {i}")).keys("ret");
    }
    let b = |t: &T| {
        let f = &t.app.comment_box.as_ref().unwrap().file;
        (f.top, f.sel.primary().cursor_line(f.text.slice(..)))
    };
    let (top, cursor) = b(&t);
    assert!(top > 10 && cursor == 20, "{top} {cursor}");
    let area = t.app.box_area;
    t.mouse(UP, area.x + 5, area.y + 2);
    assert_eq!(b(&t).0, top - 3);
    assert!(b(&t).1 < cursor, "the cursor stays in view");
    // Clicking inside the box moves its cursor to that spot.
    t.mouse(CLICK, area.x + 3, area.y + 1);
    assert_eq!(b(&t).1, top - 3);
    // The wheel over the code scrolls the code and leaves the comment unchanged.
    let body = t.app.comment_box.as_ref().unwrap().body();
    t.mouse(DOWN, 30, 2);
    assert_eq!(t.app.main_file().top, 3);
    assert_eq!(t.app.comment_box.as_ref().unwrap().body(), body);
}

#[test]
fn the_wheel_scrolls_the_help_panel_and_completions() {
    let mut t = T::files_sized(OLD, NEW, 72, 10);
    // The space menu is too long to fit on a 10-row screen.
    t.keys("space");
    let help = t.app.help_area;
    assert!(help.height > 0);
    let first = t.row(help.y as usize + 1);
    t.mouse(DOWN, help.x + 2, help.y + 2);
    assert_eq!(t.app.help_top, 3);
    assert_ne!(t.row(help.y as usize + 1), first);
    for _ in 0..20 {
        t.mouse(DOWN, help.x + 2, help.y + 2);
    }
    let last = t.app.help_top;
    t.mouse(DOWN, help.x + 2, help.y + 2);
    assert_eq!(t.app.help_top, last, "no further than the last row");
    t.keys("esc space");
    assert_eq!(t.app.help_top, 0, "a key press resets the scroll");
    assert_eq!(t.row(help.y as usize + 1), first);

    // The `:` completions take 8 rows above the statusline, and the wheel scrolls them.
    t.keys("esc :");
    let first = t.row(0);
    t.mouse(DOWN, 5, 3);
    assert_ne!(t.row(0), first);
    // The wheel anywhere else leaves the list alone. Pressing tab scrolls back to the
    // selected completion.
    let scrolled = t.row(0);
    t.mouse(DOWN, 5, 9);
    assert_eq!(t.row(0), scrolled);
    t.keys("tab");
    assert_eq!(
        t.row(0).split_whitespace().next(),
        first.split_whitespace().next()
    );
}

#[test]
fn the_wheel_scrolls_the_picker_preview() {
    let dir = tempfile::tempdir().unwrap();
    let (o, n) = (dir.path().join("old"), dir.path().join("new"));
    std::fs::create_dir_all(&o).unwrap();
    std::fs::create_dir_all(&n).unwrap();
    for f in ["a", "b"] {
        std::fs::write(o.join(f), lines(100)).unwrap();
        std::fs::write(n.join(f), format!("{}new\n", lines(50))).unwrap();
    }
    let source = Source::diff(&o, &n).unwrap();
    let mut t = T::new(source, dir, 140, 30, &config());
    t.keys("space g");
    let (list, preview) = t.app.pickers.last().unwrap().areas(t.app.screen);
    let preview = preview.expect("room for a preview");
    let shown = |t: &T| t.row(preview.y as usize + 1);
    let before = shown(&t);
    t.mouse(DOWN, preview.x + 5, preview.y + 5);
    assert_ne!(shown(&t), before);
    assert_eq!(
        t.app.pickers.last().unwrap().selected(),
        Some(&Target::Change(0))
    );
    // The wheel over the list still moves the selection, as it always has.
    t.mouse(DOWN, list.x + 5, list.y + 5);
    assert_eq!(
        t.app.pickers.last().unwrap().selected(),
        Some(&Target::Change(1))
    );
    // Changing the selection resets the preview to its centred position.
    t.keys("up");
    assert_eq!(shown(&t), before);
}

#[test]
fn sideways_scrolling() {
    let long: String = (0..30)
        .map(|i| format!("{i:02} {}\n", "abcdefghij".repeat(20)))
        .collect();
    let mut t = T::files_sized("", &long, 72, 14);
    t.keys("g g");
    let left = |t: &T| t.app.main_file().left;
    t.mouse(MouseEventKind::ScrollRight, 30, 5);
    assert_eq!(left(&t), 6);
    t.mouse_with(DOWN, 30, 5, KeyModifiers::SHIFT);
    assert_eq!(left(&t), 12, "shift and the wheel");
    t.mouse(MouseEventKind::ScrollLeft, 30, 5);
    assert_eq!(left(&t), 6);
    // Scrolling up or down, or pressing keys that don't move the cursor, keeps the code
    // scrolled sideways.
    t.mouse(DOWN, 30, 5);
    assert_eq!((left(&t), t.top()), (6, 3));
    t.keys(": esc");
    assert_eq!(left(&t), 6);
    // Moving the cursor scrolls back to show it.
    t.keys("g g");
    assert_eq!(left(&t), 0);
    // Scrolling right stops once the end of the widest line is on screen.
    for _ in 0..50 {
        t.mouse(MouseEventKind::ScrollRight, 30, 5);
    }
    let width = 72 - t.app.file().gutter_width();
    assert_eq!(left(&t), 204 - width);
    assert!(t.shows("hij"));

    // In the side-by-side view, both sides scroll together.
    t.keys("g g space v v s");
    t.mouse(MouseEventKind::ScrollRight, 60, 5);
    assert_eq!(left(&t), 6);
    // With soft wrap on, sideways scrolling does nothing.
    t.keys("space v v f g g");
    t.keys(":").typed("toggle soft-wrap").keys("ret");
    t.mouse(MouseEventKind::ScrollRight, 30, 5);
    assert_eq!(left(&t), 0);
}
