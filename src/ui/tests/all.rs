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
fn a_collapsed_file_shows_only_its_header() {
    let t = all_files(72, 30, |o, n| {
        three(o, n);
        std::fs::write(n.join("Cargo.lock"), numbered(50)).unwrap();
    });
    let s = t.screen();
    assert!(s.contains("Cargo.lock  +50 -0"), "{s}");
    assert!(s.contains("generated file, collapsed"), "{s}");
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
