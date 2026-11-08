//! Tests for all-files mode, where every changed file is in one scroll under its header.

use super::*;

/// A review of folders built by `setup`, in all-files mode, on a `w`×`h` screen.
fn all_files(w: u16, h: u16, setup: impl FnOnce(&Path, &Path)) -> T {
    let dir = tempfile::tempdir().unwrap();
    let (o, n) = (dir.path().join("old"), dir.path().join("new"));
    std::fs::create_dir_all(&o).unwrap();
    std::fs::create_dir_all(&n).unwrap();
    setup(&o, &n);
    let source = Source::diff(&o, &n).unwrap();
    let mut c = config();
    c.editor.all_files = true;
    T::new(source, dir, w, h, &c)
}

/// `n` numbered lines, `line 1` to `line n`.
fn numbered(n: usize) -> String {
    (1..=n).map(|l| format!("line {l}\n")).collect()
}

/// Three changed files: `a.rs` with one change in 30 lines, so the rest folds; `b.rs`, added;
/// and `c.rs`, with every line changed.
fn three(o: &Path, n: &Path) {
    std::fs::write(o.join("a.rs"), numbered(30)).unwrap();
    std::fs::write(
        n.join("a.rs"),
        numbered(30).replace("line 20\n", "line twenty\n"),
    )
    .unwrap();
    std::fs::write(n.join("b.rs"), "fn b() {}\n").unwrap();
    std::fs::write(o.join("c.rs"), "old\n").unwrap();
    std::fs::write(n.join("c.rs"), "new\nnewer\n").unwrap();
}

/// The cursor's file and its line on screen (1-based). A fold band is one line.
fn at(t: &T) -> (String, usize) {
    (t.path(), t.app.cursor_line() + 1)
}

#[test]
fn all_files_screen() {
    let mut t = all_files(72, 30, three);
    t.keys("g g");
    insta::assert_snapshot!(t.screen());
}

#[test]
fn a_collapsed_file_shows_its_header_and_a_summary() {
    let t = all_files(72, 30, |o, n| {
        three(o, n);
        std::fs::write(n.join("Cargo.lock"), numbered(50)).unwrap();
    });
    let s = t.screen();
    assert!(s.contains("Cargo.lock  +50 -0"), "{s}");
    assert!(s.contains("   ↕ 50 lines, generated file\n"), "{s}");
    assert!(!s.contains("enter expands"), "{s}");
    assert!(!s.contains("line 49"), "{s}");
}

#[test]
fn j_and_k_cross_into_the_next_and_previous_file() {
    let mut t = all_files(72, 30, three);
    t.keys("g g");
    assert_eq!(at(&t), ("a.rs".into(), 1));
    // The end of a.rs, then the next j goes to b.rs's first line.
    t.keys("g e");
    let end = at(&t);
    t.keys("j");
    assert_eq!(at(&t), ("b.rs".into(), 1));
    t.keys("k");
    assert_eq!(at(&t), end);
    // G is the last line of the last file, and gg comes back to the first.
    t.keys("G");
    assert_eq!(t.path(), "c.rs");
    assert!(t.app.cursor_new_line() >= 1);
    t.keys("g g");
    assert_eq!(at(&t), ("a.rs".into(), 1));
    assert_eq!(t.app.all.top, (0, 0));
}

#[test]
fn the_screen_follows_the_cursor_across_files() {
    let mut t = all_files(72, 12, |o, n| {
        for f in ["a", "b", "c", "d"] {
            std::fs::write(o.join(format!("{f}.rs")), "old\n").unwrap();
            std::fs::write(n.join(format!("{f}.rs")), numbered(8)).unwrap();
        }
    });
    t.keys("g g");
    for _ in 0..30 {
        t.keys("j");
        let line = format!("{:>3}", t.app.cursor_new_line() + 1);
        // The cursor's line is on screen, below its file's header or another file's lines.
        assert!(
            t.screen().lines().any(|l| l.contains(&line)),
            "{}",
            t.screen()
        );
    }
    assert_eq!(t.path(), "d.rs");
    assert!(!t.screen().contains("a.rs  +8 -1"), "{}", t.screen());
}

#[test]
fn page_keys_move_through_the_review() {
    let mut t = all_files(72, 12, |o, n| {
        for f in ["a", "b", "c"] {
            std::fs::write(o.join(format!("{f}.rs")), "old\n").unwrap();
            std::fs::write(n.join(format!("{f}.rs")), numbered(8)).unwrap();
        }
    });
    t.keys("g g C-d C-d C-d");
    assert_ne!(t.path(), "a.rs");
    t.keys("C-u C-u C-u C-u C-u");
    assert_eq!(at(&t), ("a.rs".into(), 1));
    t.keys("C-f C-f C-f C-f C-f");
    assert_eq!(t.path(), "c.rs");
}

#[test]
fn switching_modes_keeps_the_cursor() {
    let mut t = all_files(72, 30, three);
    t.keys("g g j j");
    let before = (at(&t), t.sel());
    t.keys("' v a");
    assert!(!t.app.all_shown());
    assert_eq!((at(&t), t.sel()), before);
    assert!(!t.screen().contains("b.rs  +1 -0"));
    t.keys("' v a");
    assert!(t.app.all_shown());
    assert_eq!((at(&t), t.sel()), before);
    assert!(t.screen().contains("b.rs  +1 -0"));
    t.keys(":").typed("one").keys("ret");
    assert!(!t.app.all_shown());
    t.keys(":").typed("all").keys("ret");
    assert!(t.app.all_shown());
}

#[test]
fn next_file_and_the_picker_go_to_a_header() {
    let mut t = all_files(72, 30, three);
    t.keys("g g g n");
    assert_eq!(at(&t), ("b.rs".into(), 1));
    assert_eq!(t.app.all.top, (1, 0));
    t.keys("space g").typed("c.rs").keys("ret");
    assert_eq!(at(&t), ("c.rs".into(), 1));
    assert!(t.screen().lines().next().unwrap().contains("c.rs  +2 -1"));
}

#[test]
fn a_comment_lands_on_the_file_under_the_cursor() {
    let mut t = all_files(72, 30, three);
    t.keys("g g g n j");
    // b.rs has one line, so j went on into c.rs.
    t.keys("x ' c").typed("on c").keys("esc ret");
    let threads = &t.app.state.threads;
    let a = threads[0].anchor.as_ref().unwrap();
    assert_eq!((a.path.as_str(), a.start), ("c.rs", 1));
    assert!(t.screen().contains("c.rs  +2 -1  1 thread"));
}

#[test]
fn the_other_files_show_their_threads() {
    let mut t = all_files(72, 30, three);
    t.keys("G x ' c").typed("last").keys("esc ret g g");
    assert_eq!(t.path(), "a.rs");
    assert!(t.screen().contains("last"), "{}", t.screen());
}

#[test]
fn an_agent_change_keeps_the_cursor_and_the_scroll() {
    let mut t = all_files(72, 30, three);
    t.keys("g g g n j");
    let (top, here) = (t.app.all.top, at(&t));
    std::fs::write(
        t.app.state.source.root.join("b.rs"),
        "fn b() {}\nfn more() {}\n",
    )
    .unwrap();
    t.app.files_changed();
    t.draw();
    assert_eq!(at(&t), here);
    assert_eq!(t.app.all.top, top);
    assert!(t.screen().contains("b.rs  +2 -0"));
}

#[test]
fn a_file_that_is_no_longer_changed_hands_over_to_the_next() {
    let mut t = all_files(72, 30, three);
    t.keys("g g g n");
    assert_eq!(t.path(), "b.rs");
    let root = t.app.state.source.root.clone();
    std::fs::remove_file(root.join("b.rs")).unwrap();
    t.app.files_changed();
    t.draw();
    assert_eq!(at(&t), ("c.rs".into(), 1));
    assert_eq!(t.status(), Some("b.rs has no changes now"));
}

#[test]
fn all_files_is_off_unless_set() {
    let t = T::folders(three);
    assert!(!t.app.all_shown());
    assert!(!t.screen().contains("b.rs  +1 -0"));
}

#[test]
fn other_views_show_one_file() {
    let mut t = all_files(72, 30, three);
    t.keys("' v u");
    assert!(!t.app.all_shown());
    t.keys("' v f");
    assert!(t.app.all_shown());
}

#[test]
fn far_files_are_dropped_and_read_again() {
    let mut t = all_files(72, 12, |o, n| {
        for i in 0..80 {
            std::fs::write(o.join(format!("f{i:02}.rs")), "old\n").unwrap();
            std::fs::write(n.join(format!("f{i:02}.rs")), "new\n").unwrap();
        }
    });
    for _ in 0..80 {
        t.keys("g n");
    }
    let loaded = t.app.state.files.iter().filter(|f| f.is_some()).count();
    assert!(
        loaded <= super::super::all::MAX_LOADED,
        "{loaded} files loaded"
    );
    t.keys("g g");
    assert_eq!(at(&t), ("f00.rs".into(), 1));
    assert!(t.screen().contains("f00.rs  +1 -1"));
}

/// Four files of 8 new lines each, on a screen `h` rows high.
fn four(h: u16) -> T {
    all_files(72, h, |o, n| {
        for f in ["a", "b", "c", "d"] {
            std::fs::write(o.join(format!("{f}.rs")), "old\n").unwrap();
            std::fs::write(n.join(format!("{f}.rs")), numbered(8)).unwrap();
        }
    })
}

#[test]
fn the_wheel_scrolls_through_every_file() {
    let mut t = four(12);
    t.keys("g g");
    for _ in 0..20 {
        t.mouse(MouseEventKind::ScrollDown, 30, 5);
    }
    // The wheel went past the end of a.rs, and the cursor came along to stay on screen.
    assert_eq!(t.path(), "d.rs");
    assert!(!t.screen().contains("a.rs  +8 -1"), "{}", t.screen());
    for _ in 0..20 {
        t.mouse(MouseEventKind::ScrollUp, 30, 5);
    }
    assert_eq!(t.app.all.top, (0, 0));
    assert_eq!(t.path(), "a.rs");
}

#[test]
fn the_wheel_leaves_the_cursor_while_it_is_on_screen() {
    // The cursor is in the middle of the screen, on b.rs's fifth line.
    let mut t = four(30);
    t.keys("g g");
    t.keys(&"j ".repeat(12));
    let before = at(&t);
    assert_eq!(before, ("b.rs".into(), 5));
    t.mouse(MouseEventKind::ScrollDown, 30, 5);
    assert_eq!(at(&t), before);
    assert_ne!(t.app.all.top, (0, 0));
}

/// The screen row showing `text`.
fn row_of(t: &T, text: &str) -> u16 {
    t.screen()
        .lines()
        .position(|l| l.contains(text))
        .unwrap_or_else(|| panic!("{text} isn't on screen:\n{}", t.screen())) as u16
}

#[test]
fn a_click_lands_in_the_file_under_it() {
    let mut t = four(30);
    t.keys("g g");
    // b.rs's header and its first lines are on screen below a.rs.
    let y = row_of(&t, "b.rs  +8 -1") + 3;
    t.mouse(MouseEventKind::Down(MouseButton::Left), 12, y);
    assert_eq!(at(&t), ("b.rs".into(), 3));
    // A click in a later file goes there, and a click back in an earlier file goes there.
    let y = row_of(&t, "c.rs  +8 -1") + 2;
    t.mouse(MouseEventKind::Down(MouseButton::Left), 12, y);
    assert_eq!(at(&t), ("c.rs".into(), 2));
    let y = row_of(&t, "b.rs  +8 -1") + 1;
    t.mouse(MouseEventKind::Down(MouseButton::Left), 12, y);
    assert_eq!(at(&t), ("b.rs".into(), 1));
}

#[test]
fn files_are_set_apart_by_a_blank_row_and_a_rule() {
    let t = all_files(72, 30, three);
    let s = t.screen();
    let lines: Vec<&str> = s.lines().collect();
    let b = row_of(&t, "▼ b.rs") as usize;
    assert_eq!(lines[b - 1].trim(), "", "{s}");
    assert!(lines[b].starts_with("── ▼ b.rs  +1 -0 ──"), "{s}");
    assert!(lines[b].trim_end().ends_with('─'), "{s}");
    // The first file's header is on the top row, with no blank row above it.
    assert!(lines[0].starts_with("── ▼ a.rs"), "{s}");
}

#[test]
fn a_click_on_a_header_collapses_and_expands_the_file_and_keeps_its_folds() {
    let mut t = all_files(72, 60, three);
    // Open a.rs's folds, then move on to c.rs.
    t.keys("g g z R G");
    assert!(!t.screen().contains("↕ lines 1–16"));
    let here = at(&t);
    let y = row_of(&t, "a.rs  +1 -1");
    t.mouse(MouseEventKind::Down(MouseButton::Left), 20, y);
    let s = t.screen();
    assert!(s.contains("── ▶ a.rs  +1 -1"), "{s}");
    assert!(!s.contains("line twenty"), "{s}");
    // The cursor stays where it was.
    assert_eq!(at(&t), here);
    let y = row_of(&t, "a.rs  +1 -1");
    t.mouse(MouseEventKind::Down(MouseButton::Left), 20, y);
    let s = t.screen();
    assert!(s.contains("── ▼ a.rs  +1 -1"), "{s}");
    // The folds are still open.
    assert!(!s.contains("↕ lines 1–16"), "{s}");
    assert!(s.contains("line twenty"), "{s}");
    // A click on a collapsed file's summary opens it too.
    let y = row_of(&t, "a.rs  +1 -1");
    t.mouse(MouseEventKind::Down(MouseButton::Left), 20, y);
    let y = row_of(&t, "↕ 30 lines");
    t.mouse(MouseEventKind::Down(MouseButton::Left), 20, y);
    assert!(t.screen().contains("── ▼ a.rs"), "{}", t.screen());
    assert_eq!(at(&t), here);
}

#[test]
fn ret_collapses_and_expands_the_cursors_file() {
    let mut t = all_files(72, 30, three);
    t.keys("g g j j");
    let before = (at(&t), t.sel());
    t.keys("ret");
    let s = t.screen();
    assert!(s.contains("── ▶ a.rs"), "{s}");
    assert!(!s.contains("line 17"), "{s}");
    // The summary says how many lines the file has, without repeating how to open it.
    let summary = s.lines().nth(1).unwrap();
    assert_eq!(summary.trim_end(), "   ↕ 30 lines", "{s}");
    assert_eq!(row_of(&t, "▼ b.rs"), 3, "{s}");
    t.keys("ret");
    let s = t.screen();
    assert!(s.contains("── ▼ a.rs"), "{s}");
    assert!(s.contains("↕ lines 1–16"), "{s}");
    assert_eq!((at(&t), t.sel()), before);
    // j from a collapsed file goes on into the next one.
    t.keys("ret j");
    assert_eq!(t.path(), "b.rs");
    t.keys("k");
    assert_eq!(t.path(), "a.rs");
}

#[test]
fn the_header_of_the_file_at_the_top_stays_pinned() {
    let mut t = four(12);
    t.keys("g g");
    t.keys(&"j ".repeat(14));
    assert_eq!(t.path(), "b.rs");
    let top = t.app.all.top;
    assert!(top.1 > 0, "{top:?}");
    let s = t.screen();
    let first = s.lines().next().unwrap();
    assert!(
        first.starts_with(&format!("── ▼ {}.rs", ["a", "b", "c", "d"][top.0])),
        "{s}"
    );
    // The cursor's line is still on screen, below the pinned header.
    let line = format!("{:>5} ", t.app.cursor_new_line() + 1);
    assert!(s.lines().skip(1).any(|l| l.starts_with(&line)), "{s}");
    // A click on the pinned header collapses its file.
    t.mouse(MouseEventKind::Down(MouseButton::Left), 20, 0);
    assert!(
        t.screen()
            .contains(&format!("── ▶ {}.rs", ["a", "b", "c", "d"][top.0]))
    );
}

#[test]
fn a_headers_counts_are_coloured_as_added_and_removed() {
    let t = all_files(72, 30, three);
    // The header row's cells, one symbol each, so a position is a column.
    let row: Vec<String> = (0..72).map(|x| t.cell(x, 0).symbol().to_string()).collect();
    let col = |text: &str| {
        let first = text.chars().next().unwrap().to_string();
        let second = text.chars().nth(1).unwrap().to_string();
        (0..71)
            .find(|&x| row[x] == first && row[x + 1] == second)
            .unwrap() as u16
    };
    let (plus, minus) = (t.app.theme.get("diff.plus"), t.app.theme.get("diff.minus"));
    assert_eq!(t.cell(col("+1"), 0).fg, plus.fg.unwrap());
    assert_eq!(t.cell(col("-1"), 0).fg, minus.fg.unwrap());
    assert_ne!(t.cell(col("a."), 0).fg, plus.fg.unwrap());
}
