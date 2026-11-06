//! The app driven as a user drives it: keys and mouse events in, then the selection (in Helix's
//! `#[h|]#ello` notation), the view and the screen checked. Screen snapshots lock the parts that
//! must look like Helix.

use std::{collections::HashSet, path::Path};

use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use glos_helix::test::plain;
use ratatui::{Terminal, backend::TestBackend, style::Color};
use tempfile::TempDir;

use super::{prompt::completions, *};

/// An app over a review, with a test terminal of a fixed size.
struct T {
    app: App,
    term: Terminal<TestBackend>,
    _dir: TempDir,
}

const OLD: &str = "fn main() {\n    println!(\"hi\");\n}\n\nfn gone() {}\n";
const NEW: &str = "fn main() {\n    println!(\"hello\");\n    let x = 1;\n}\n";

/// The defaults, but without the file tree, so screens and clicks are about the file view;
/// `the_file_tree_starts_open` tests the default.
fn config() -> Config {
    let mut c = Config::default();
    c.editor.file_tree = false;
    c
}

impl T {
    /// Two files, `old.rs` → `main.rs`.
    fn files(old: &str, new: &str) -> T {
        Self::files_sized(old, new, 72, 14)
    }

    fn files_sized(old: &str, new: &str, w: u16, h: u16) -> T {
        let dir = tempfile::tempdir().unwrap();
        let (o, n) = (dir.path().join("old.rs"), dir.path().join("main.rs"));
        std::fs::write(&o, old).unwrap();
        std::fs::write(&n, new).unwrap();
        let source = Source::diff(&o, &n).unwrap();
        Self::new(source, dir, w, h, &config())
    }

    /// Two folders built by `setup(old, new)`.
    fn folders(setup: impl FnOnce(&Path, &Path)) -> T {
        let dir = tempfile::tempdir().unwrap();
        let (o, n) = (dir.path().join("old"), dir.path().join("new"));
        std::fs::create_dir_all(&o).unwrap();
        std::fs::create_dir_all(&n).unwrap();
        setup(&o, &n);
        let source = Source::diff(&o, &n).unwrap();
        Self::new(source, dir, 72, 14, &config())
    }

    fn new(source: Source, dir: TempDir, w: u16, h: u16, config: &Config) -> T {
        let theme = Theme::load("default", ColorSupport::TrueColor).unwrap();
        let conn = crate::store::open(&dir.path().join("glos.db")).unwrap();
        let app = App::new(source, config, theme, conn).unwrap();
        let mut t = T {
            app,
            term: Terminal::new(TestBackend::new(w, h)).unwrap(),
            _dir: dir,
        };
        t.draw();
        t
    }

    fn draw(&mut self) {
        let app = &mut self.app;
        self.term.draw(|f| app.draw(f)).unwrap();
    }

    /// Helix-notation keys separated by spaces (`g`, `space`, `C-o`, `ret`…), drawing after
    /// each as the event loop does.
    fn keys(&mut self, keys: &str) -> &mut Self {
        for k in keys.split_whitespace() {
            self.app
                .key(Key::parse(k).unwrap_or_else(|| panic!("key {k}")));
            self.draw();
        }
        self
    }

    /// Types text into a prompt or picker.
    fn typed(&mut self, text: &str) -> &mut Self {
        for c in text.chars() {
            self.app.key(Key {
                code: KeyCode::Char(c),
                mods: KeyModifiers::NONE,
            });
            self.draw();
        }
        self
    }

    fn mouse(&mut self, kind: MouseEventKind, column: u16, row: u16) -> &mut Self {
        self.app.mouse(MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        });
        self.draw();
        self
    }

    /// The current file and selection in Helix's notation.
    fn sel(&self) -> String {
        let f = self.app.file();
        plain(f.text.clone(), &f.sel)
    }

    fn top(&self) -> usize {
        self.app.file().top
    }

    fn status(&self) -> Option<&str> {
        self.app.status.as_ref().map(|(s, _)| s.as_str())
    }

    fn path(&self) -> String {
        match &self.app.state.cur {
            Cur::Change(i) => self.app.state.source.changes[*i].path.clone(),
            Cur::Context(p) => format!("context:{p}"),
        }
    }

    fn screen(&self) -> String {
        let buf = self.term.backend().buffer();
        (0..buf.area.height)
            .map(|y| {
                (0..buf.area.width)
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn cell(&self, x: u16, y: u16) -> &ratatui::buffer::Cell {
        &self.term.backend().buffer()[(x, y)]
    }
}

/// `n` numbered lines, `line 1` to `line n`.
fn lines(n: usize) -> String {
    (1..=n).map(|i| format!("line {i}\n")).collect()
}

// ---- Screens ----

#[test]
fn file_view() {
    insta::assert_snapshot!(T::files(OLD, NEW).screen());
}

#[test]
fn key_help_panel() {
    insta::assert_snapshot!(T::files(OLD, NEW).keys("g").screen());
}

#[test]
fn select_mode_and_count() {
    insta::assert_snapshot!(T::files(OLD, NEW).keys("v 2 w").screen());
}

#[test]
fn changed_file_picker() {
    insta::assert_snapshot!(T::files(OLD, NEW).keys("space g").screen());
}

#[test]
fn search_prompt() {
    insta::assert_snapshot!(T::files(OLD, NEW).keys("/").typed("let").screen());
}

#[test]
fn picker_with_preview() {
    let t = T::folders(|o, n| {
        std::fs::write(o.join("a.rs"), "fn a() {}\n").unwrap();
        std::fs::write(n.join("a.rs"), "fn a() { changed }\n").unwrap();
        std::fs::write(n.join("b.md"), "# new\n").unwrap();
    });
    let mut t = t;
    t.term = Terminal::new(TestBackend::new(100, 16)).unwrap();
    insta::assert_snapshot!(t.keys("space g").screen());
}

#[test]
fn notices_for_kinds_of_change() {
    let mut t = T::folders(|o, n| {
        std::fs::write(o.join("gone.txt"), "bye\n").unwrap();
        std::fs::write(o.join("before.txt"), "same\n").unwrap();
        std::fs::write(n.join("after.txt"), "same\n").unwrap();
        std::fs::write(n.join("Cargo.lock"), "lock\n").unwrap();
        std::fs::write(n.join("bin.dat"), b"\x00\x01binary").unwrap();
        std::os::unix::fs::symlink("after.txt", n.join("link")).unwrap();
        std::fs::write(o.join("run.sh"), "echo\n").unwrap();
        std::fs::write(n.join("run.sh"), "echo\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(n.join("run.sh"), std::fs::Permissions::from_mode(0o755)).unwrap();
    });
    let mut seen = vec![];
    for _ in 0..t.app.state.files.len() {
        seen.push(format!(
            "{}: {}",
            t.path(),
            t.app.file().notices.join(" | ")
        ));
        t.keys("g n");
    }
    insta::assert_snapshot!(seen.join("\n"));
}

/// Colours, which the text snapshots don't see: the primary cursor and selection, the gutter's
/// diff signs and the selected line number, all from the theme as Helix picks them.
#[test]
fn styles_follow_the_theme() {
    let mut t = T::files(OLD, NEW);
    t.keys("g g x");
    let theme = &t.app.theme;
    let (_, sel_primary, _, cursor_primary) = t.app.selection_styles();
    let gutter = t.app.file().gutter_width() as u16;
    // Line 1 is selected by `x`: its text has the primary selection's background…
    assert_eq!(t.cell(gutter, 0).bg, sel_primary.bg.unwrap_or(Color::Reset));
    // …and the cursor sits on the newline at its end.
    let end = gutter + "fn main() {".len() as u16;
    let c = t.cell(end, 0);
    assert_eq!(c.bg, cursor_primary.bg.unwrap_or(c.bg));
    assert!(c.modifier.contains(cursor_primary.add_modifier) || cursor_primary.bg.is_some());
    // The line number of the cursor's line is `ui.linenr.selected`, the others `ui.linenr`.
    assert_eq!(
        t.cell(4, 0).fg,
        theme.get("ui.linenr.selected").fg.unwrap_or(Color::Reset)
    );
    assert_eq!(
        t.cell(4, 3).fg,
        theme.get("ui.linenr").fg.unwrap_or(Color::Reset)
    );
    // Lines 2–3 replace one old line, so they're one modified hunk; line 5 shows a removal.
    let sign = |y| {
        (
            t.cell(gutter - 1, y).symbol().to_string(),
            t.cell(gutter - 1, y).fg,
        )
    };
    let fg = |scope| theme.get(scope).fg.unwrap_or(Color::Reset);
    assert_eq!(sign(1), ("▍".into(), fg("diff.delta.gutter")));
    assert_eq!(sign(2), ("▍".into(), fg("diff.delta.gutter")));
    assert_eq!(sign(4), ("▔".into(), fg("diff.minus.gutter")));
    assert_eq!(sign(0).0, " ");
}

// ---- Keys ----

#[test]
fn counts() {
    let mut t = T::files("", &lines(30));
    assert_eq!(
        t.keys("3 j")
            .app
            .file()
            .sel
            .primary()
            .cursor_line(t.app.file().text.slice(..)),
        3
    );
    // Two digits make one count; a leading 0 isn't a count (and 0 isn't bound).
    t.keys("g g 1 2 j");
    assert_eq!(
        t.app
            .file()
            .sel
            .primary()
            .cursor_line(t.app.file().text.slice(..)),
        12
    );
    t.keys("g g 0 j");
    assert_eq!(
        t.app
            .file()
            .sel
            .primary()
            .cursor_line(t.app.file().text.slice(..)),
        1
    );
    // The count is used once.
    t.keys("g g 2 j j");
    assert_eq!(
        t.app
            .file()
            .sel
            .primary()
            .cursor_line(t.app.file().text.slice(..)),
        3
    );
}

#[test]
fn menus_and_unbound_keys() {
    let mut t = T::files("", "ab\ncd\n");
    t.keys("g");
    assert_eq!(t.app.pending.len(), 1);
    // An unbound key drops the sequence; the next key works normally.
    t.keys("X");
    assert!(t.app.pending.is_empty());
    assert_eq!(t.keys("j").sel(), "ab\n#[c|]#d\n");
    assert_eq!(t.keys("g g").sel(), "#[a|]#b\ncd\n");
    // A count survives into a menu.
    assert_eq!(t.keys("2 g g").sel(), "ab\n#[c|]#d\n");
}

#[test]
fn next_key_commands() {
    let mut t = T::files("", "a b a b a\n");
    assert_eq!(t.keys("2 f a").sel(), "#[a b a b a|]#\n");
    assert_eq!(t.keys("g h t b").sel(), "#[a |]#b a b a\n");
    let mut t = T::files("", "f(a, b)\n");
    assert_eq!(t.keys("l l m i (").sel(), "f(#[a, b|]#)\n");
    // esc instead of the character does nothing.
    assert_eq!(t.keys("f esc").sel(), "f(#[a, b|]#)\n");
    assert!(t.app.next_key.is_none());
}

#[test]
fn select_mode() {
    let mut t = T::files("", "one two three\n");
    assert_eq!(t.keys("v w w").sel(), "#[one two |]#three\n");
    assert!(t.app.state.mode == Mode::Select);
    assert!(t.keys("esc").app.state.mode == Mode::Normal);
    assert!(t.keys("v v").app.state.mode == Mode::Normal);
}

#[test]
fn search_previews_and_esc_restores() {
    let mut t = T::files("", &lines(60));
    t.keys("/").typed("line 50");
    assert_eq!(
        t.app
            .file()
            .sel
            .primary()
            .cursor_line(t.app.file().text.slice(..)),
        49
    );
    assert!(t.top() > 0);
    t.keys("esc");
    assert_eq!(t.sel(), format!("#[l|]#{}", &lines(60)[1..]));
    assert_eq!(t.top(), 0);
    assert!(t.app.last_search.is_none());
}

#[test]
fn search_next_prev_and_star() {
    let mut t = T::files("", "foo bar foo baz foo\n");
    t.keys("/").typed("foo").keys("ret");
    assert_eq!(t.sel(), "foo bar #[foo|]# baz foo\n");
    assert_eq!(t.keys("n").sel(), "foo bar foo baz #[foo|]#\n");
    // Wraps around, and says so.
    assert_eq!(t.keys("n").sel(), "#[foo|]# bar foo baz foo\n");
    assert_eq!(t.status(), Some("Wrapped around document"));
    assert_eq!(t.keys("N").sel(), "foo bar foo baz #[foo|]#\n");
    // An empty search repeats the last.
    assert_eq!(t.keys("/ ret n").sel(), "#[foo|]# bar foo baz foo\n");
    // `*` searches the selected word.
    t.keys("g h f b m i w *");
    assert_eq!(t.app.last_search.as_deref(), Some("\\bbar\\b"));
    assert_eq!(t.keys("g g n").sel(), "foo #[bar|]# foo baz foo\n");
}

#[test]
fn search_without_history_or_matches() {
    let mut t = T::files("", "abc\n");
    t.keys("n");
    assert_eq!(t.status(), Some("No previous search"));
    t.keys("/").typed("zzz").keys("ret");
    assert_eq!(t.sel(), "#[a|]#bc\n");
    t.keys("n");
    assert_eq!(t.status(), Some("No more matches"));
    t.keys("/").typed("(").keys("ret");
    assert_eq!(t.status(), Some("Invalid regex"));
}

#[test]
fn regex_prompts() {
    let mut t = T::files("", "a1 b2 c3\n");
    t.keys("x s").typed("\\d");
    assert_eq!(t.app.file().sel.len(), 3);
    // esc puts the selection back as it was.
    t.keys("esc");
    assert_eq!(t.sel(), "#[a1 b2 c3\n|]#");
    t.keys("s").typed("\\d").keys("ret");
    // helix-core keeps the first match primary.
    assert_eq!(t.sel(), "a#[1|]# b#(2|)# c#(3|)#\n");
    t.keys("K").typed("[12]").keys("ret");
    assert_eq!(t.app.file().sel.len(), 2);
    t.keys("A-K").typed("1").keys("ret");
    assert_eq!(t.app.file().sel.len(), 1);
    assert!(t.sel().contains("#[2|]#"), "{}", t.sel());
    // Nothing matching leaves the selection and says why.
    t.keys("x s").typed("z").keys("ret");
    assert_eq!(t.status(), Some("nothing selected"));
}

#[test]
fn sticky_view_menu() {
    let mut t = T::files_sized("", &lines(100), 72, 14);
    t.keys("Z j j");
    assert_eq!(t.top(), 2);
    assert_eq!(t.app.pending.len(), 1, "Z stays open");
    t.keys("esc");
    assert!(t.app.pending.is_empty());
    // z is not sticky.
    t.keys("z j");
    assert!(t.app.pending.is_empty());
    assert_eq!(t.top(), 3);
}

#[test]
fn align_view() {
    let mut t = T::files_sized("", &lines(100), 72, 14);
    t.keys("5 0 G z t");
    assert_eq!(t.top(), 49);
    t.keys("z b");
    assert_eq!(t.top(), 49 - 11);
    t.keys("z z");
    assert_eq!(t.top(), 49 - 5);
}

#[test]
fn jumplist() {
    let mut t = T::files_sized("", &lines(100), 72, 14);
    t.keys("1 0 j g e");
    let line = |t: &T| {
        t.app
            .file()
            .sel
            .primary()
            .cursor_line(t.app.file().text.slice(..))
    };
    assert_eq!(line(&t), 99);
    t.keys("g g");
    assert_eq!(line(&t), 0);
    t.keys("C-o");
    assert_eq!(line(&t), 99);
    t.keys("C-o");
    assert_eq!(line(&t), 10);
    // Nothing older: stays.
    t.keys("C-o");
    assert_eq!(line(&t), 10);
    t.keys("C-i");
    assert_eq!(line(&t), 99);
    t.keys("tab");
    assert_eq!(line(&t), 0);
    // C-s saves the current place.
    t.keys("5 j C-s g e C-o");
    assert_eq!(line(&t), 5);
}

#[test]
fn repeat_last_motion() {
    let mut t = T::files("", "a.b.c.d\n");
    // As in Helix, a repeated `f` selects from the cursor, so it takes in the `.` it's on.
    t.keys("f .");
    assert_eq!(t.sel(), "#[a.|]#b.c.d\n");
    assert_eq!(t.keys("A-.").sel(), "a#[.b.|]#c.d\n");
    // A count repeats it; past the last match it stays.
    assert_eq!(t.keys("2 A-.").sel(), "a.b#[.c.|]#d\n");
}

#[test]
fn macros() {
    let mut t = T::files("", "a b c d e f\n");
    t.keys("Q w Q");
    assert_eq!(t.status(), Some("Recorded to register [@]"));
    assert_eq!(t.sel(), "#[a |]#b c d e f\n");
    assert_eq!(t.keys("q").sel(), "a #[b |]#c d e f\n");
    assert_eq!(t.keys("2 q").sel(), "a b c #[d |]#e f\n");
}

#[test]
fn changed_files_and_context() {
    let mut t = T::folders(|o, n| {
        for f in ["a", "b", "c"] {
            std::fs::write(o.join(f), "old\n").unwrap();
            std::fs::write(n.join(f), "new\n").unwrap();
        }
        std::fs::write(o.join("same"), "same\n").unwrap();
        std::fs::write(n.join("same"), "same\n").unwrap();
    });
    assert_eq!(t.path(), "a");
    assert_eq!(t.keys("g n").path(), "b");
    assert_eq!(t.keys("2 g n").path(), "a", "counts wrap around");
    assert_eq!(t.keys("g p").path(), "c");
    // space f opens an unchanged file for context; g n goes back to the changes.
    t.keys("space f").typed("same").keys("ret");
    assert_eq!(t.path(), "context:same");
    assert_eq!(t.app.file().notices, ["not changed; shown for context"]);
    assert_eq!(t.keys("g n").path(), "a");
    // A changed file chosen in the file picker opens as its change.
    t.keys("space f").typed("b").keys("ret");
    assert_eq!(t.path(), "b");
    // esc closes a picker without moving.
    t.keys("space g esc");
    assert!(t.app.pickers.is_empty());
    assert_eq!(t.path(), "b");
}

#[test]
fn collapsed_files() {
    let mut t = T::folders(|_, n| std::fs::write(n.join("Cargo.lock"), "a\nb\n").unwrap());
    assert!(t.app.file().collapsed);
    // Motions do nothing while collapsed; enter expands.
    t.keys("j");
    assert_eq!(t.sel(), "#[a|]#\nb\n");
    t.keys("ret j");
    assert!(!t.app.file().collapsed);
    assert_eq!(t.sel(), "a\n#[b|]#\n");
    assert!(t.keys("ret").app.file().collapsed);
    // enter on an ordinary file does nothing.
    let mut t = T::files("", "x\n");
    assert!(!t.keys("ret").app.file().collapsed);
}

#[test]
fn typed_commands() {
    let mut t = T::files("", &lines(20));
    t.keys(":").typed("7").keys("ret");
    assert_eq!(
        t.app
            .file()
            .sel
            .primary()
            .cursor_line(t.app.file().text.slice(..)),
        6
    );
    t.keys(":").typed("nope").keys("ret");
    assert_eq!(t.status(), Some("no such command: 'nope'"));
    t.keys(":").typed("toggle soft-wrap").keys("ret");
    assert!(t.app.state.wrap);
    t.keys(":").typed("toggle nonsense").keys("ret");
    assert_eq!(t.status(), Some("unknown option 'nonsense'"));
    // Backspace on an empty prompt closes it; esc closes it too.
    t.keys(": backspace");
    assert!(t.app.prompt.is_none());
    t.keys(":").typed("q").keys("esc");
    assert!(!t.app.quit);
    t.keys(":").typed("q").keys("ret");
    assert!(t.app.quit);
}

#[test]
fn yank() {
    let mut t = T::files("", "one two\n");
    t.keys("v w y");
    assert_eq!(t.app.yanked, ["one "]);
    assert_eq!(t.status(), Some("yanked 1 selection to register \""));
    assert!(
        t.app.state.mode == Mode::Normal,
        "yanking leaves select mode"
    );
}

// ---- Scrolling and the mouse ----

#[test]
fn view_follows_the_cursor_with_scrolloff() {
    // 14 rows: 12 for text.
    let mut t = T::files_sized("", &lines(100), 72, 14);
    t.keys("6 j");
    assert_eq!(t.top(), 0);
    t.keys("j");
    assert_eq!(t.top(), 1, "the cursor stays 5 lines from the bottom");
    t.keys("g e");
    assert_eq!(t.top(), 99 - 12 + 5 + 1);
    t.keys("g g");
    assert_eq!(t.top(), 0);
    // A search that lands outside the view centres it.
    t.keys("/").typed("line 60$").keys("ret");
    assert_eq!(t.top(), 59 - 6);
}

#[test]
fn horizontal_scroll() {
    let long = format!("{}end\n", "x".repeat(100));
    let mut t = T::files_sized("", &long, 40, 6);
    t.keys("g l");
    assert!(t.app.file().left > 0);
    assert!(t.screen().contains("end"));
    t.keys("g h");
    assert_eq!(t.app.file().left, 0);
}

#[test]
fn soft_wrap_counts_rows() {
    let mut text = String::new();
    for i in 0..30 {
        text += &format!("{i} {}\n", "word ".repeat(20));
    }
    let mut t = T::files_sized("", &text, 40, 14);
    t.keys(": t o g g l e space s o f t - w r a p ret");
    assert!(t.app.state.wrap);
    assert!(t.screen().contains("↪"));
    // `j` moves by visual row: still within line 0.
    t.keys("j");
    assert_eq!(
        t.app
            .file()
            .sel
            .primary()
            .cursor_line(t.app.file().text.slice(..)),
        0
    );
    // Each line wraps to several rows, so moving four lines (`gj`) already scrolls.
    t.keys("4 g j");
    assert!(t.top() > 0, "top {}", t.top());
    let cursor_line = t
        .app
        .file()
        .sel
        .primary()
        .cursor_line(t.app.file().text.slice(..));
    assert!(t.screen().contains(&format!("{cursor_line} word")));
}

#[test]
fn page_and_half_page() {
    let mut t = T::files_sized("", &lines(100), 72, 14);
    t.keys("C-f");
    assert_eq!(t.top(), 12);
    t.keys("C-b");
    assert_eq!(t.top(), 0);
    // C-b left the cursor 5 lines above the bottom (line 6); C-d moves view and cursor half a
    // page.
    t.keys("C-d");
    assert_eq!(t.top(), 6);
    assert_eq!(
        t.app
            .file()
            .sel
            .primary()
            .cursor_line(t.app.file().text.slice(..)),
        12
    );
}

#[test]
fn mouse_click_drag_and_wheel() {
    let mut t = T::files_sized("", &lines(100), 72, 14);
    let gutter = t.app.file().gutter_width() as u16;
    // Click line 3, column 2.
    t.mouse(MouseEventKind::Down(MouseButton::Left), gutter + 2, 2);
    assert!(
        t.sel().starts_with("line 1\nline 2\nli#[n|]#e 3\n"),
        "{}",
        &t.sel()[..40]
    );
    // Drag to line 4 extends.
    t.mouse(MouseEventKind::Drag(MouseButton::Left), gutter + 1, 3);
    // The cursor takes in the character under it, as in Helix.
    assert!(
        t.sel().starts_with("line 1\nline 2\nli#[ne 3\nli|]#ne 4\n"),
        "{}",
        &t.sel()[..40]
    );
    // Clicks in the gutter do nothing.
    let before = t.sel();
    t.mouse(MouseEventKind::Down(MouseButton::Left), 1, 5);
    assert_eq!(t.sel(), before);
    // The wheel scrolls three lines; the cursor is kept in view.
    t.mouse(MouseEventKind::ScrollDown, 10, 5);
    t.mouse(MouseEventKind::ScrollDown, 10, 5);
    assert_eq!(t.top(), 6);
    assert!(
        t.app
            .file()
            .sel
            .primary()
            .cursor_line(t.app.file().text.slice(..))
            >= 6 + 5
    );
}

#[test]
fn mouse_below_notices_and_in_picker() {
    let mut t = T::folders(|o, n| {
        std::fs::write(o.join("a"), "x\n").unwrap();
        std::fs::write(n.join("a"), "x\n").unwrap();
        std::fs::write(n.join("b"), "one\ntwo\n").unwrap();
    });
    // `b` is new: one notice row above its text.
    t.keys("g n");
    assert_eq!(t.path(), "b");
    let gutter = t.app.file().gutter_width() as u16;
    t.mouse(MouseEventKind::Down(MouseButton::Left), gutter, 2);
    assert_eq!(t.sel(), "one\n#[t|]#wo\n");
    // In the picker a click opens the row under it.
    t.keys("space g");
    let (list, _) = t.app.pickers.last().unwrap().areas(t.app.screen);
    t.mouse(
        MouseEventKind::Down(MouseButton::Left),
        list.x + 4,
        list.y + 4,
    );
    assert!(t.app.pickers.is_empty());
    assert_eq!(t.path(), "b");
    // The command line and the comment box swallow clicks: the cursor stays.
    let click = |t: &mut T| {
        t.mouse(MouseEventKind::Down(MouseButton::Left), gutter + 2, 2);
    };
    t.keys(":");
    click(&mut t);
    assert_eq!(t.sel(), "one\n#[t|]#wo\n");
    assert!(t.app.prompt.is_some());
    t.keys("esc space c");
    click(&mut t);
    assert!(t.app.comment_box.is_some());
    t.keys("esc q");
    assert_eq!(t.sel(), "one\n#[t|]#wo\n");
}

#[test]
fn mouse_off_in_config_is_still_safe() {
    // The setting only stops glos capturing the mouse; events that arrive anyway are harmless.
    let mut c = config();
    c.editor.mouse = false;
    let dir = tempfile::tempdir().unwrap();
    let (o, n) = (dir.path().join("o"), dir.path().join("n"));
    std::fs::write(&o, "a\n").unwrap();
    std::fs::write(&n, "b\n").unwrap();
    let mut t = T::new(Source::diff(&o, &n).unwrap(), dir, 40, 8, &c);
    t.mouse(MouseEventKind::ScrollDown, 0, 0);
    assert_eq!(t.top(), 0);
}

// ---- Commenting ----

impl T {
    /// The status line's text, or "".
    fn said(&self) -> String {
        self.status().unwrap_or("").to_string()
    }

    fn bodies(&self) -> Vec<String> {
        self.app
            .state
            .threads
            .iter()
            .flat_map(|t| t.comments.iter().map(|c| c.body.clone()))
            .collect()
    }

    /// The help panel's text, as a screen row search.
    fn shows(&self, text: &str) -> bool {
        self.screen().contains(text)
    }
}

#[test]
fn comment_box_screen() {
    let mut t = T::files(OLD, NEW);
    t.keys("x space c").typed("rename `x`");
    insta::assert_snapshot!(t.screen());
    t.keys("esc ret");
    // Saved, the thread shows as one line under its line, with a mark in the gutter.
    insta::assert_snapshot!("thread_marker", t.screen());
}

#[test]
fn commenting_on_a_selection_saves_at_once() {
    let mut t = T::files(OLD, NEW);
    // The cursor opens on the first change, line 2; `x` then `x` selects lines 2–3.
    t.keys("x x space c");
    assert!(
        t.app.comment_box.as_ref().unwrap().insert,
        "opens in insert mode"
    );
    assert!(t.shows("Comment on main.rs:2-3"), "{}", t.screen());
    assert!(t.shows(" INS "));
    t.typed("why").keys("ret").typed("not hi?");
    // esc leaves insert mode; ret saves.
    t.keys("esc");
    assert!(t.shows(" NOR "));
    t.keys("ret");
    assert!(t.app.comment_box.is_none());
    assert_eq!(t.bodies(), ["why\nnot hi?"]);
    let a = t.app.state.threads[0].anchor.clone().unwrap();
    assert_eq!((a.path.as_str(), a.start, a.end), ("main.rs", 2, 3));
    assert_eq!(t.said(), "Comment saved · 1 unsent; space s submits");
    assert!(t.shows("1 unsent"), "{}", t.screen());
    // It's in the store, so a new app on the same database sees it.
    let n = review::unsent(&t.app.conn, &t.app.review.id).unwrap();
    assert_eq!(n, 1);
}

#[test]
fn drafts_survive_closing_the_box() {
    let mut t = T::files(OLD, NEW);
    t.keys("space c").typed("half a thou");
    t.keys("esc q");
    assert!(t.app.comment_box.is_none());
    assert!(t.said().starts_with("Draft kept"), "{}", t.said());
    assert!(t.bodies().is_empty(), "nothing is sent by closing");
    // Back on the same line, the draft returns, cursor at its end.
    t.keys("space c");
    assert_eq!(t.said(), "Draft restored");
    t.typed("ght").keys("esc ret");
    assert_eq!(t.bodies(), ["half a thought"]);
    // Saved, the draft is gone.
    t.keys("space c");
    assert_eq!(t.app.comment_box.as_ref().unwrap().body(), "");
    // An empty box saves nothing.
    t.keys("esc ret");
    assert_eq!(t.said(), "Nothing to save");
    assert_eq!(t.bodies().len(), 1);
}

#[test]
fn motions_work_in_the_box_and_files_stay_put() {
    let mut t = T::files(OLD, NEW);
    t.keys("space c").typed("one two three").keys("esc");
    // Normal-mode motions act on the box's text, as in Helix.
    t.keys("b b");
    assert_eq!(t.sel(), "one #[|two ]#three");
    t.keys("i").typed("big ").keys("esc");
    assert_eq!(
        t.app.comment_box.as_ref().unwrap().body(),
        "one big two three"
    );
    // Changing file, pickers and submitting wait until the box closes.
    t.keys("g n");
    assert!(t.said().starts_with("Close the comment box first"));
    t.keys("space g");
    assert!(t.app.pickers.is_empty());
    // `:` commands still work, and `:q` with a draft just quits: the draft is saved.
    t.keys(":").typed("q").keys("ret");
    assert!(t.app.quit);
}

#[test]
fn threads_open_and_close() {
    let mut t = T::files(OLD, NEW);
    t.keys("space c")
        .typed("first line\nsecond")
        .keys("esc ret");
    assert!(t.shows("▸ you: first line · unsent"), "{}", t.screen());
    assert!(!t.shows("second"));
    // ret on the thread's line opens it…
    t.keys("ret");
    assert!(t.shows("▾ thread"), "{}", t.screen());
    assert!(t.shows("    second"));
    // …and closes it again.
    t.keys("ret");
    assert!(!t.shows("second"));
    // A click on the marker opens it too.
    let y = t.app.thread_hits[0].0;
    t.mouse(MouseEventKind::Down(MouseButton::Left), 10, y);
    assert!(t.shows("    second"));
}

#[test]
fn long_comments_wrap_to_the_screen() {
    // 72 columns, less the gutter: a comment longer than that wraps instead of being cut off.
    let mut t = T::files(OLD, NEW);
    let body = "a long comment that goes on well past the right edge of the screen, \
                so it has to wrap onto more rows to be read in full";
    t.keys("space c").typed(body).keys("esc ret");
    let width = |l: &str| unicode_width::UnicodeWidthStr::width(l);
    let screen = t.screen();
    assert!(screen.lines().all(|l| width(l) <= 72), "{screen}");
    // Closed, it takes two rows and says there's more.
    assert!(screen.contains('…'), "{screen}");
    t.keys("ret");
    let screen = t.screen();
    assert!(t.shows("in full"), "the end shows once open: {screen}");
    assert!(screen.lines().all(|l| width(l) <= 72), "{screen}");
    // Every row of it is one the view counts, so the status bar stays put below them.
    assert!(screen.lines().last().unwrap().contains("NOR"), "{screen}");
    // A click on any of its rows closes it again.
    let y = t.app.thread_hits.last().unwrap().0;
    t.mouse(MouseEventKind::Down(MouseButton::Left), 10, y);
    assert!(!t.shows("in full"), "{}", t.screen());
}

#[test]
fn opened_threads_count_when_scrolling() {
    let body = (1..=8)
        .map(|i| format!("l{i}"))
        .collect::<Vec<_>>()
        .join("\n");
    let mut t = T::files("", &lines(40));
    t.keys("g g space c").typed(&body).keys("esc ret ret");
    assert_eq!(t.top(), 0);
    // Nine thread rows sit under line 1: line 4 is already low in a 12-row view.
    t.keys("3 j");
    assert!(t.top() > 0, "the cursor was scrolled into view");
    let screen = t.screen();
    let cursor_row = screen.lines().position(|l| l.contains("line 4")).unwrap();
    assert!(cursor_row + 5 < 12, "scrolloff kept:\n{screen}");
}

#[test]
fn editing_and_deleting_a_comment() {
    let mut t = T::files(OLD, NEW);
    t.keys("space c").typed("tpyo").keys("esc ret");
    t.keys("space m");
    assert!(t.shows("Edit comment"));
    assert_eq!(t.app.comment_box.as_ref().unwrap().body(), "tpyo");
    t.keys("backspace backspace backspace backspace")
        .typed("typo")
        .keys("esc ret");
    assert_eq!(t.bodies(), ["typo"]);
    // Saving it empty asks once, then deletes it.
    t.keys("space m");
    for _ in 0..4 {
        t.keys("backspace");
    }
    t.keys("esc ret");
    assert!(t.said().contains("deletes the comment"));
    assert!(t.app.comment_box.is_some());
    t.keys("ret");
    assert_eq!(t.said(), "Comment deleted");
    assert!(t.app.state.threads.is_empty());
    // With no thread here, there's nothing to edit or reply to.
    t.keys("space m");
    assert_eq!(t.said(), "No comment of yours here");
    t.keys("space r");
    assert_eq!(t.said(), "No thread here to reply to");
}

#[test]
fn space_shift_m_twice_deletes_your_latest_unsent_comment() {
    let mut t = T::files(OLD, NEW);
    t.keys("space c").typed("first").keys("esc ret");
    t.keys("space r").typed("second").keys("esc ret");
    t.keys("space M");
    assert_eq!(t.said(), "space M again deletes your comment: «second»");
    assert_eq!(t.bodies(), ["first", "second"]);
    t.keys("space M");
    assert_eq!(t.said(), "Comment deleted");
    assert_eq!(t.bodies(), ["first"]);
    assert_eq!(t.app.unsent, 1);
    // The last comment goes, and the thread with it.
    t.keys("space M space M");
    assert!(t.app.state.threads.is_empty());
    assert_eq!(t.app.unsent, 0);
}

#[test]
fn any_other_key_forgets_a_delete() {
    let mut t = T::files(OLD, NEW);
    t.keys("space c").typed("keep me").keys("esc ret");
    t.keys("space M");
    // esc out of the space menu is a key too.
    t.keys("space esc space M");
    assert!(
        t.said().starts_with("space M again deletes"),
        "{}",
        t.said()
    );
    t.keys("l space M");
    assert!(
        t.said().starts_with("space M again deletes"),
        "{}",
        t.said()
    );
    assert_eq!(t.bodies(), ["keep me"]);
}

#[test]
fn a_long_comment_is_quoted_by_its_first_line() {
    let mut t = T::files(OLD, NEW);
    let long = "a first line that runs on for well over forty characters";
    t.keys("space c")
        .typed(long)
        .keys("ret")
        .typed("more")
        .keys("esc ret");
    t.keys("space M");
    assert_eq!(
        t.said(),
        "space M again deletes your comment: «a first line that runs on for well over…»"
    );
    t.keys("esc");
    t.keys("space r")
        .typed("short")
        .keys("ret")
        .typed("two")
        .keys("esc ret");
    t.keys("space M");
    assert_eq!(t.said(), "space M again deletes your comment: «short…»");
}

#[test]
fn the_typed_command_deletes_too() {
    let mut t = T::files(OLD, NEW);
    t.keys("space c").typed("gone").keys("esc ret");
    t.typed(":delete-comment").keys("ret");
    assert!(
        t.said().starts_with("space M again deletes"),
        "{}",
        t.said()
    );
    t.typed(":delete-comment").keys("ret");
    assert_eq!(t.said(), "Comment deleted");
    assert!(t.app.state.threads.is_empty());
}

#[test]
fn a_sent_comment_is_not_deleted() {
    let mut t = T::files(OLD, NEW);
    t.keys("space c").typed("sent").keys("esc ret");
    t.keys("space s c");
    t.keys("space M");
    assert_eq!(
        t.said(),
        "That comment was already sent to the agent: edit it with space m"
    );
    t.keys("space M");
    assert_eq!(t.bodies(), ["sent"]);
    // An older unsent comment is not reached past the sent one.
    let mut t = T::files(OLD, NEW);
    t.keys("space c").typed("old").keys("esc ret");
    let old = t.app.state.threads[0].comments[0].id.clone();
    t.keys("space r").typed("new").keys("esc ret");
    t.keys("space s c");
    // Editing a sent comment makes it unsent again. The UI only edits the latest, so the
    // store edits the older one here.
    crate::review::edit(&t.app.conn, &old, "old, edited").unwrap();
    t.app.reload();
    assert!(t.app.state.threads[0].comments[0].unsent());
    t.keys("space M space M");
    assert!(
        t.said().starts_with("That comment was already sent"),
        "{}",
        t.said()
    );
    assert_eq!(t.bodies(), ["old, edited", "new"]);
}

#[test]
fn no_comment_of_yours_to_delete() {
    let mut t = T::files(OLD, NEW);
    t.keys("space M");
    assert_eq!(t.said(), "No comment of yours here");
    assert!(t.app.state.threads.is_empty());
}

#[test]
fn space_shift_m_waits_for_the_comment_box() {
    let mut t = T::files(OLD, NEW);
    t.keys("space c").typed("one").keys("esc ret");
    t.keys("space r").typed("draft").keys("esc space M");
    assert!(
        t.said().starts_with("Close the comment box first"),
        "{}",
        t.said()
    );
    assert_eq!(t.bodies(), ["one"]);
}

#[test]
fn the_palette_finds_delete_comment() {
    let mut t = T::files(OLD, NEW);
    t.keys("C-g").typed("delete");
    assert!(!t.shows("Delete your latest comment"), "no thread here yet");
    t.keys("esc");
    t.keys("space c").typed("one").keys("esc ret");
    t.keys("C-g").typed("delete");
    assert!(t.shows("space M"), "{}", t.screen());
    assert!(t.shows("Delete your latest comment"), "{}", t.screen());
}

#[test]
fn replies_and_review_level_comments() {
    let mut t = T::files(OLD, NEW);
    t.keys("space c").typed("one").keys("esc ret");
    t.keys("space r").typed("two").keys("esc ret");
    assert_eq!(t.said(), "Reply saved · 2 unsent; space s submits");
    assert_eq!(t.app.state.threads.len(), 1);
    assert_eq!(t.bodies(), ["one", "two"]);
    t.keys("space C");
    assert!(t.shows("Comment on the review"));
    t.typed("overall fine").keys("esc ret");
    assert_eq!(t.app.state.threads.len(), 2);
    assert!(t.app.state.threads[0].anchor.is_none());
}

#[test]
fn no_comments_on_deleted_or_binary_files() {
    let mut t = T::folders(|o, n| {
        std::fs::write(o.join("gone.rs"), "bye\n").unwrap();
        std::fs::write(n.join("bin.dat"), b"\x00\x01").unwrap();
    });
    assert_eq!(t.path(), "bin.dat");
    t.keys("space c");
    assert!(t.app.comment_box.is_none());
    assert!(t.said().contains("binary"));
    t.keys("g n space c");
    assert_eq!(t.path(), "gone.rs");
    assert!(t.said().contains("deleted"), "{}", t.said());
}

#[test]
fn the_final_empty_line_comments_on_the_last_line() {
    let mut t = T::files("a\n", "a\nb\n");
    t.keys("g e space c");
    assert!(t.shows("Comment on main.rs:2 "), "{}", t.screen());
}

#[test]
fn submitting_with_a_verdict() {
    let mut t = T::files(OLD, NEW);
    // With nothing to send, only approving applies, and the help panel says so.
    t.keys("space s");
    assert!(t.shows("Approve"));
    assert!(!t.shows("Request changes"), "{}", t.screen());
    t.keys("esc");
    t.keys("space s r");
    assert!(t.said().starts_with("nothing to submit"), "{}", t.said());
    t.keys("space c").typed("fix").keys("esc ret");
    t.keys("space s");
    assert!(t.shows("Request changes") && t.shows("Send comments without a verdict"));
    t.keys("r");
    assert_eq!(
        t.said(),
        "Round 1 submitted: changes requested, 1 comment. The agent gets it from wait_for_review"
    );
    assert_eq!(t.app.unsent, 0);
    assert!(!t.shows("unsent"));
    // What was reviewed is kept: every file's new side.
    let snap: i64 = t
        .app
        .conn
        .query_row(
            "SELECT count(*) FROM snapshot_files WHERE round = 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(snap, 1);
    // An approval with nothing new is an LGTM.
    t.keys(":").typed("submit approve").keys("ret");
    assert!(
        t.said()
            .starts_with("Round 2 submitted: approved, 0 comments")
    );
    t.keys(":").typed("submit maybe").keys("ret");
    assert!(t.said().starts_with("Submit how?"));
}

#[test]
fn quitting_with_unsent_comments_reminds_once() {
    let mut t = T::files(OLD, NEW);
    t.keys("space c").typed("x").keys("esc ret");
    t.keys(":").typed("q").keys("ret");
    assert!(!t.app.quit);
    assert!(t.said().starts_with("1 comment not sent"), "{}", t.said());
    t.keys(":").typed("q").keys("ret");
    assert!(t.app.quit);
    let mut t = T::files(OLD, NEW);
    t.keys("space c").typed("x").keys("esc ret");
    t.keys(":").typed("q!").keys("ret");
    assert!(t.app.quit);
}

#[test]
fn thread_motions_and_pickers() {
    let mut t = T::files("", &lines(20));
    for line in [3, 9, 15] {
        t.keys(":").typed(&line.to_string());
        t.keys("ret space c")
            .typed(&format!("at {line}"))
            .keys("esc ret");
    }
    t.keys("g g ] d");
    assert!(
        t.sel().contains("#[line 3\n|]#"),
        "selects the thread's lines"
    );
    t.keys("] d ] d");
    assert!(t.sel().contains("#[line 15\n|]#"));
    t.keys("] d");
    assert_eq!(t.said(), "No more threads in this file");
    t.keys("[ d");
    assert!(t.sel().contains("#[line 9\n|]#"));
    t.keys("[ D");
    assert!(t.sel().contains("#[line 3\n|]#"));
    t.keys("] D");
    assert!(t.sel().contains("#[line 15\n|]#"));
    // The whole review's thread picker, filtered, opens a thread on its lines and opens it.
    t.keys("space D").typed("at 9").keys("ret");
    assert!(t.sel().contains("#[line 9\n|]#"));
    assert!(t.shows("▾ thread"));
}

#[test]
fn a_review_level_thread_opens_its_comment_from_the_picker() {
    let mut t = T::files(OLD, NEW);
    t.keys("space C").typed("overall").keys("esc ret");
    t.keys("space D ret");
    assert!(t.shows("Edit comment"), "{}", t.screen());
    assert_eq!(t.app.comment_box.as_ref().unwrap().body(), "overall");
}

#[test]
fn help_lists_only_what_applies() {
    let mut t = T::files(OLD, NEW);
    t.keys("space");
    assert!(t.shows("Comment on the selected lines"));
    assert!(!t.shows("Reply to the thread"), "no thread here yet");
    t.keys("esc space c").typed("x").keys("esc ret space");
    assert!(t.shows("Reply to the thread"));
    assert!(t.shows("Edit your latest comment"));
    // Off the thread's line, the thread actions go again.
    t.keys("esc g g space");
    assert!(!t.shows("Reply to the thread"));
}

#[test]
fn command_completion() {
    let mut t = T::files(OLD, NEW);
    t.keys(":");
    // Every command is listed with what it does.
    assert!(t.shows("submit") && t.shows("Comment on the whole review"));
    t.typed("su").keys("tab");
    assert_eq!(t.app.prompt.as_ref().unwrap().line, "submit");
    t.typed(" ").keys("tab");
    assert_eq!(t.app.prompt.as_ref().unwrap().line, "submit approve");
    t.keys("tab");
    assert_eq!(t.app.prompt.as_ref().unwrap().line, "submit changes");
    t.keys("S-tab S-tab");
    assert_eq!(t.app.prompt.as_ref().unwrap().line, "submit comment");
    // Aliases find their command.
    let (_, items) = completions("qa");
    assert_eq!(
        items.iter().map(|i| i.0.as_str()).collect::<Vec<_>>(),
        ["quit", "quit!"]
    );
    assert!(completions("nope").1.is_empty());
    assert_eq!(completions("toggle s").1[0].0, "soft-wrap");
}

#[test]
fn a_request_from_another_process_shows_without_moving_the_view() {
    let mut t = T::files("", &lines(40));
    t.keys("2 0 j");
    let (sel, top) = (t.sel(), t.top());
    // An MCP server writes to the same database.
    let other = crate::store::open(&t._dir.path().join("glos.db")).unwrap();
    t.app
        .review
        .request(&other, "claude", "ready: added the parser", &[])
        .unwrap();
    t.app.refresh();
    assert_eq!(
        t.said(),
        "claude asks for a review: ready: added the parser"
    );
    assert_eq!((t.sel(), t.top()), (sel, top));
}

// ---- Two-way: agents, live files, re-anchoring ----

impl T {
    /// Another process on the same store, as an agent's MCP server is.
    fn agent(&self) -> rusqlite::Connection {
        crate::store::open(&self._dir.path().join("glos.db")).unwrap()
    }

    /// Writes the new side of a two-file review, as an agent editing it would.
    fn edit(&mut self, text: &str) -> &mut Self {
        std::fs::write(self._dir.path().join("main.rs"), text).unwrap();
        self.app.files_changed();
        self.draw();
        self
    }

    fn thread_lines(&self) -> Vec<(u32, u32, String)> {
        self.app
            .state
            .threads
            .iter()
            .filter_map(|t| {
                t.anchor
                    .as_ref()
                    .map(|a| (a.start, a.end, t.status.clone()))
            })
            .collect()
    }
}

/// A review with one submitted comment on line 2 of `NEW`, and the thread's id.
fn sent(t: &mut T) -> String {
    t.keys("space c")
        .typed("why hello?")
        .keys("esc ret space s r");
    t.app.state.threads[0].id.clone()
}

#[test]
fn agent_replies_appear_unread_and_read_when_opened() {
    let mut t = T::files(OLD, NEW);
    let id = sent(&mut t);
    review::reply(&t.agent(), &id, "claude", "it reads better").unwrap();
    t.app.refresh();
    t.draw();
    assert_eq!(t.said(), "New reply from the agent · ]u goes to it");
    assert!(t.shows("1 unread"), "{}", t.screen());
    assert!(t.shows("· unread · 2 comments"), "{}", t.screen());
    // Opening the thread reads it.
    t.keys("ret");
    assert!(t.shows("    it reads better"));
    assert!(t.app.state.threads.iter().all(|th| !th.unread()));
    assert!(!t.shows("1 unread"), "{}", t.screen());
    let stored = review::threads(&t.agent(), &t.app.review.id).unwrap();
    assert!(!stored[0].unread(), "read is kept in the store");
}

#[test]
fn next_unread_goes_across_files() {
    let mut t = T::folders(|o, n| {
        for f in ["a.rs", "b.rs"] {
            std::fs::write(o.join(f), "1\n").unwrap();
            std::fs::write(n.join(f), "1\n2\n3\n").unwrap();
        }
    });
    let agent = t.agent();
    let file = "1\n2\n3\n";
    let at = |p: &str, l| Anchor {
        path: p.into(),
        start: l,
        end: l,
    };
    review::annotate(
        &agent,
        &t.app.review.id,
        &at("b.rs", 3),
        file,
        "claude",
        "unsure",
    )
    .unwrap();
    review::annotate(
        &agent,
        &t.app.review.id,
        &at("a.rs", 2),
        file,
        "claude",
        "why",
    )
    .unwrap();
    t.app.refresh();
    t.keys("g g");
    assert_eq!(t.path(), "a.rs");
    t.keys("] u");
    assert_eq!(
        (t.path().as_str(), t.sel().contains("#[2\n|]#")),
        ("a.rs", true)
    );
    assert!(
        t.shows("claude (note)"),
        "an agent's note is marked:\n{}",
        t.screen()
    );
    t.keys("] u");
    assert_eq!(t.path(), "b.rs");
    assert!(t.sel().contains("#[3\n|]#"));
    t.keys("] u");
    assert_eq!(t.said(), "No more unread replies");
    assert!(t.app.state.threads.iter().all(|th| !th.unread()));
    assert!(!t.shows("1 unread"), "{}", t.screen());
}

#[test]
fn resolve_and_reopen() {
    let mut t = T::files(OLD, NEW);
    t.keys("space c").typed("x").keys("esc ret");
    t.keys("space x");
    assert_eq!(t.said(), "Thread resolved");
    assert_eq!(t.app.state.threads[0].status, "resolved");
    assert!(t.shows("· resolved"));
    t.keys("space x");
    assert_eq!(t.said(), "Thread reopened");
    assert_eq!(t.app.state.threads[0].status, "open");
    t.keys("g g space x");
    assert_eq!(t.said(), "No thread here");
}

#[test]
fn suggestions_show_and_space_n_goes_there() {
    let mut t = T::files("", &lines(40));
    t.keys("g g");
    let (sel, top) = (t.sel(), t.top());
    t.app
        .review
        .suggest(&t.agent(), "claude", "main.rs", 30, "the loop")
        .unwrap();
    t.app.refresh();
    assert_eq!(
        t.said(),
        "claude suggests main.rs:30: the loop · space n goes there"
    );
    assert_eq!((t.sel(), t.top()), (sel, top), "the view didn't move");
    t.keys("space n");
    assert!(t.sel().contains("#[line 30\n|]#"));
    // Back where you were with C-o.
    t.keys("C-o");
    assert_eq!(t.top(), top);
}

#[test]
fn files_changing_keep_your_place_and_move_threads() {
    let mut t = T::files("", &lines(30));
    t.keys(":")
        .typed("20")
        .keys("ret space c")
        .typed("this one")
        .keys("esc ret");
    t.keys("5 l");
    let before = t.sel();
    assert!(before.contains("line #[2|]#0"), "{before}");
    let top = t.top();
    // The agent adds two lines at the top.
    t.edit(&format!("new 1\nnew 2\n{}", lines(30)));
    assert!(
        t.sel().contains("line #[2|]#0"),
        "the cursor stays on the same code"
    );
    assert_eq!(t.top(), top + 2, "and so does the view");
    assert_eq!(t.thread_lines(), [(22, 22, "open".into())]);
    assert!(t.shows("new 1") || t.top() > 0);
}

#[test]
fn threads_go_outdated_only_once_files_settle() {
    let mut t = T::files("", &lines(10));
    t.keys(":")
        .typed("5")
        .keys("ret space c")
        .typed("hmm")
        .keys("esc ret");
    t.edit(&lines(10).replace("line 5\n", "line five\n"));
    // Mid-edit: still open where it was.
    assert_eq!(t.thread_lines(), [(5, 5, "open".into())]);
    // Settled: outdated, and opening it shows the code it was on.
    t.app.reanchor(true);
    t.draw();
    assert_eq!(t.thread_lines(), [(5, 5, "outdated".into())]);
    t.keys(":").typed("5").keys("ret ret");
    assert!(
        t.shows("the code it was on:") && t.shows("│ line 5"),
        "{}",
        t.screen()
    );
    // Reopened and the text back, it's placed again.
    t.edit(&lines(10));
    t.keys("space x");
    assert_eq!(t.thread_lines(), [(5, 5, "open".into())]);
}

#[test]
fn files_appear_and_stop_being_changed() {
    let mut t = T::folders(|o, n| {
        std::fs::write(o.join("a.rs"), "1\n").unwrap();
        std::fs::write(n.join("a.rs"), "1\n2\n").unwrap();
    });
    let new = t._dir.path().join("new");
    std::fs::write(new.join("b.rs"), "fresh\n").unwrap();
    t.app.files_changed();
    t.draw();
    assert_eq!(t.app.state.source.changes.len(), 2);
    assert_eq!(t.path(), "a.rs");
    assert!(t.shows("1/2 files"));
    // a.rs changed back: it stays on screen, for context.
    std::fs::write(new.join("a.rs"), "1\n").unwrap();
    t.app.files_changed();
    t.draw();
    assert_eq!(t.path(), "context:a.rs");
    t.keys("g n");
    assert_eq!(t.path(), "b.rs");
    // Everything changed back: nothing left to step through.
    std::fs::remove_file(new.join("b.rs")).unwrap();
    t.app.files_changed();
    t.keys("g n");
    assert_eq!(t.said(), "No changed files left");
}

#[test]
fn threads_follow_changes_made_while_glos_was_closed() {
    let mut t = T::files("", &lines(10));
    t.keys(":")
        .typed("3")
        .keys("ret space c")
        .typed("x")
        .keys("esc ret");
    let dir = t._dir.path().to_path_buf();
    std::fs::write(dir.join("main.rs"), format!("top\n{}", lines(10))).unwrap();
    // A new glos on the same store places the thread again at start.
    let source = Source::diff(&dir.join("old.rs"), &dir.join("main.rs")).unwrap();
    let theme = Theme::load("default", ColorSupport::TrueColor).unwrap();
    let app = App::new(source, &config(), theme, t.agent()).unwrap();
    let a = app.state.threads[0].anchor.clone().unwrap();
    assert_eq!((a.start, a.end), (4, 4));
}

// ---- Views, scopes, the file tree ----

/// A git repo with `main` and a branch `topic` of two commits, then uncommitted work.
fn git_repo() -> T {
    git_repo_with(|r| Source::git(r, None).unwrap())
}

fn git_repo_with(source: impl FnOnce(&Path) -> Source) -> T {
    let dir = tempfile::tempdir().unwrap();
    let r = dir.path().join("repo");
    std::fs::create_dir_all(&r).unwrap();
    let git = |args: &[&str]| {
        let out = std::process::Command::new("git")
            .args([
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "-c",
                "init.defaultBranch=main",
            ])
            .args(args)
            .current_dir(&r)
            .output()
            .unwrap();
        assert!(out.status.success(), "{out:?}");
    };
    git(&["init", "-q"]);
    std::fs::write(r.join("a.rs"), lines(10)).unwrap();
    git(&["add", "."]);
    git(&["commit", "-qm", "base"]);
    git(&["checkout", "-qb", "topic"]);
    std::fs::write(r.join("a.rs"), lines(10).replace("line 2\n", "line two\n")).unwrap();
    git(&["commit", "-qam", "two"]);
    std::fs::write(r.join("b.rs"), "fn b() {}\n").unwrap();
    git(&["add", "."]);
    git(&["commit", "-qm", "add b"]);
    std::fs::write(r.join("c.rs"), "uncommitted\n").unwrap();
    let source = source(&r);
    T::new(source, dir, 72, 14, &config())
}

#[test]
fn unified_view_screen() {
    let mut t = T::files(OLD, NEW);
    t.keys("space v u");
    insta::assert_snapshot!(t.screen());
    // Removed and added lines differ by colour, which the text snapshot doesn't see.
    let sign = t.app.main_file().gutter_width() as u16 - 1;
    let fg = |name: &str| t.app.theme.get(name).fg.unwrap_or(Color::Reset);
    assert_eq!(t.cell(sign, 1).fg, fg("diff.minus.gutter"));
    assert_eq!(t.cell(sign, 2).fg, fg("diff.plus.gutter"));
    assert_ne!(fg("diff.minus.gutter"), fg("diff.plus.gutter"));
}

#[test]
fn split_view_screen() {
    let mut t = T::files(OLD, NEW);
    t.keys("space v s");
    insta::assert_snapshot!(t.screen());
}

#[test]
fn views_keep_your_place() {
    let mut t = T::files("", &lines(40));
    t.keys(":").typed("30").keys("ret");
    let line = |t: &T| t.app.cursor_new_line();
    assert_eq!(line(&t), 29);
    for v in ["u", "s", "f", "u", "f"] {
        t.keys(&format!("space v {v}"));
        assert_eq!(line(&t), 29, "view {v}");
    }
    assert!(
        t.sel().contains("#[l|]#ine 30"),
        "back in the file, on the same line"
    );
}

#[test]
fn unified_folds_and_comments_go_on_the_new_side() {
    let old = lines(20);
    let new = old.replace("line 10\n", "line ten\n");
    let mut t = T::files(&old, &new);
    t.keys("space v u");
    assert!(t.shows("↕ lines 1–6 (6)"), "{}", t.screen());
    assert!(t.shows(" unified"));
    // The cursor opens on the change; the line above it is the removed one.
    assert_eq!(t.app.cursor_new_line(), 9);
    t.keys("k space c");
    assert!(
        t.said().starts_with("Comments go on the new side"),
        "{}",
        t.said()
    );
    // Selecting the removed and added lines comments on the added one.
    t.keys("x x space c");
    assert!(t.shows("Comment on main.rs:10 "), "{}", t.screen());
    t.typed("ten?").keys("esc ret");
    assert_eq!(t.thread_lines(), [(10, 10, "open".into())]);
    // The thread shows under its line in the view, and in the file view too.
    assert!(t.shows("▸ you: ten?"));
    let row = |t: &T, s: &str| t.screen().lines().position(|l| l.contains(s)).unwrap();
    assert_eq!(row(&t, "▸ you"), row(&t, "line ten") + 1);
    t.keys("space v f");
    assert_eq!(row(&t, "▸ you"), row(&t, "line ten") + 1);
}

#[test]
fn split_keeps_the_sides_level_with_threads() {
    let mut t = T::files("a\nb\nc\n", "a\nB\nnew\nc\n");
    t.keys("space v s");
    t.keys("g g j space c").typed("x").keys("esc ret");
    let screen = t.screen();
    let lines: Vec<&str> = screen.lines().collect();
    // The thread row on the right has a blank row on the left: `c` is level on both sides.
    let c_row: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, l)| l.matches(" c").count() == 2)
        .map(|(i, _)| i)
        .collect();
    assert_eq!(c_row.len(), 1, "{screen}");
    assert!(lines[2].contains("▸ you: x"), "{screen}");
    assert!(!t.app.wrap(), "side by side never wraps");
}

#[test]
fn since_the_last_review() {
    let mut t = T::folders(|o, n| {
        for f in ["a.rs", "b.rs"] {
            std::fs::write(o.join(f), "1\n").unwrap();
            std::fs::write(n.join(f), "1\n2\n").unwrap();
        }
    });
    t.keys("space o r");
    assert!(t.said().starts_with("Nothing submitted yet"));
    assert!(!t.shows("Since your last review"));
    t.keys("space s a");
    // The agent changes b.rs after the review.
    std::fs::write(t._dir.path().join("new/b.rs"), "1\n2\n3\n").unwrap();
    t.app.files_changed();
    t.keys("space o r");
    assert_eq!(t.said(), "Since round 1: 1 file");
    assert_eq!(t.path(), "b.rs");
    assert!(t.shows("Since round 1"), "{}", t.screen());
    // Only line 3 is new since the review.
    assert_eq!(t.app.base_file().hunks.len(), 1);
    assert_eq!(t.app.base_file().hunks[0].after, 2..3);
    t.keys("space o b");
    assert_eq!(t.said(), "Old vs new: 2 files");
}

#[test]
fn one_commit_at_a_time() {
    let mut t = git_repo();
    assert_eq!(t.app.state.source.changes.len(), 3);
    // `]C` only shows in the per-commit scope.
    t.keys("]");
    assert!(!t.shows("Goto next commit"));
    t.keys("esc space o c");
    assert!(t.said().starts_with("Commit 1/2"), "{}", t.said());
    assert_eq!(t.path(), "a.rs");
    assert!(t.shows("two"), "{}", t.screen());
    t.keys("] C");
    assert!(t.said().contains("Commit 2/2") && t.said().contains("add b"));
    assert_eq!(t.path(), "b.rs");
    t.keys("] C");
    assert_eq!(t.said(), "The last commit");
    // Comments work on a commit too.
    t.keys("space c");
    assert!(t.app.comment_box.is_some());
    t.keys("esc q");
    t.keys("[ C [ C");
    assert_eq!(t.said(), "The first commit");
    t.keys("space o b");
    assert!(t.said().starts_with("Branch vs base: 3 files"));
}

#[test]
fn filtering_files() {
    let mut t = T::folders(|o, n| {
        for f in ["a.rs", "b.md", "c.rs"] {
            std::fs::write(o.join(f), "1\n").unwrap();
            std::fs::write(n.join(f), "2\n").unwrap();
        }
    });
    t.keys(":").typed("filter *.rs").keys("ret");
    assert_eq!(t.said(), "Old vs new, files *.rs: 2 files");
    t.keys("g n g n");
    assert_eq!(t.path(), "a.rs", "b.md is left out");
    t.keys("space c").typed("x").keys("esc ret g n");
    t.keys(":").typed("filter threads").keys("ret");
    assert_eq!(t.said(), "Old vs new, files with open threads: 1 file");
    t.keys(":").typed("filter").keys("ret");
    assert_eq!(t.said(), "Old vs new: 3 files");
    t.keys(":").typed("filter [").keys("ret");
    assert!(t.said().starts_with("Bad glob"));
}

/// Two folders, `a/` and `b/`, with a file each, and the tree open but not focused.
fn two_folder_tree() -> T {
    let mut t = T::folders(|o, n| {
        for d in ["a", "b"] {
            std::fs::create_dir_all(o.join(d)).unwrap();
            std::fs::create_dir_all(n.join(d)).unwrap();
            std::fs::write(n.join(d).join("x.rs"), format!("{d}\n")).unwrap();
        }
    });
    t.keys("space e esc");
    t
}

#[test]
fn opening_a_file_in_a_closed_folder_opens_the_folder() {
    let mut t = two_folder_tree();
    assert_eq!(t.path(), "a/x.rs");
    t.app.tree.as_mut().unwrap().collapsed.insert("b/".into());
    t.draw();
    assert_eq!(t.screen().matches("x.rs").count(), 2, "{}", t.screen());
    t.keys("g n");
    assert_eq!(t.path(), "b/x.rs");
    let tree = t.app.tree.as_ref().unwrap();
    assert!(tree.collapsed.is_empty(), "b/ opened");
    assert_eq!(
        tree.selected(),
        Some("b/x.rs"),
        "the unfocused tree follows"
    );
    // Through a picker too.
    t.app.tree.as_mut().unwrap().collapsed.insert("a/".into());
    t.keys("space g").typed("a/x").keys("ret");
    assert_eq!(t.path(), "a/x.rs");
    let tree = t.app.tree.as_ref().unwrap();
    assert!(tree.collapsed.is_empty());
    assert_eq!(tree.selected(), Some("a/x.rs"));
}

#[test]
fn a_focused_tree_keeps_its_cursor_when_the_file_changes() {
    let mut t = two_folder_tree();
    t.keys("space e");
    // The file changes without the tree's help, as it does after a suggestion or a jump.
    t.app.tree.as_mut().unwrap().collapsed.insert("b/".into());
    t.app.open(Target::Path("b/x.rs".into()));
    t.draw();
    let tree = t.app.tree.as_ref().unwrap();
    assert!(tree.collapsed.is_empty(), "the file's folder opens");
    assert_eq!(tree.selected(), Some("a/x.rs"), "the cursor stays");
}

#[test]
fn a_rebuild_keeps_closed_folders_closed() {
    let mut t = two_folder_tree();
    t.app.tree.as_mut().unwrap().collapsed.insert("b/".into());
    let new = t._dir.path().join("new");
    std::fs::write(new.join("b/y.rs"), "y\n").unwrap();
    t.app.files_changed();
    t.draw();
    assert_eq!(t.app.state.source.changes.len(), 3);
    let tree = t.app.tree.as_ref().unwrap();
    assert!(tree.collapsed.contains("b/"));
    assert!(!t.shows("y.rs"), "{}", t.screen());
}

/// A focused tree over `README.md`, `src/main.rs`, `src/ui/mod.rs`, `src/ui/view.rs` and
/// `tests/mcp.rs`.
fn nested_tree() -> T {
    let mut t = T::folders(|_, n| {
        for f in [
            "README.md",
            "src/main.rs",
            "src/ui/mod.rs",
            "src/ui/view.rs",
            "tests/mcp.rs",
        ] {
            let p = n.join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, "x\n").unwrap();
        }
    });
    t.keys("space e");
    t
}

impl T {
    /// The path under the tree's cursor, a folder's included.
    fn tree_at(&self) -> String {
        let tree = self.app.tree.as_ref().unwrap();
        tree.rows[tree.cursor].path.clone()
    }

    fn closed(&mut self, folders: &[&str]) -> &mut Self {
        let tree = self.app.tree.as_mut().unwrap();
        tree.collapsed = folders.iter().map(|f| f.to_string()).collect();
        self.draw();
        self
    }
}

#[test]
fn tree_with_a_closed_folder() {
    let mut t = nested_tree();
    t.closed(&["tests/"]);
    insta::assert_snapshot!(t.screen());
}

#[test]
fn the_palette_finds_closing_the_tree() {
    let mut t = T::files(OLD, NEW);
    t.keys("C-g").typed("close the file tree");
    assert!(
        t.shows("Open, focus or close the file tree"),
        "{}",
        t.screen()
    );
}

#[test]
fn searching_the_tree_jumps_as_you_type() {
    let mut t = nested_tree();
    assert_eq!(t.tree_at(), "README.md");
    t.keys("/");
    assert!(t.shows("search:"));
    t.typed("m");
    assert_eq!(t.tree_at(), "src/main.rs", "the first name with an m");
    t.typed("o");
    assert_eq!(t.tree_at(), "src/ui/mod.rs");
    // esc puts the cursor back.
    t.keys("esc");
    assert_eq!(t.tree_at(), "README.md");
    assert!(t.app.prompt.is_none());
}

#[test]
fn tree_search_matches_only_the_shown_name() {
    let mut t = nested_tree();
    t.keys("/").typed("src").keys("ret");
    assert_eq!(t.tree_at(), "src/");
    t.keys("n");
    assert_eq!(t.tree_at(), "src/", "no file's name has src in it");
}

#[test]
fn tree_search_opens_closed_folders_holding_the_match() {
    let mut t = nested_tree();
    t.closed(&["src/", "src/ui/", "tests/"]);
    t.keys("/").typed("view").keys("ret");
    assert_eq!(t.tree_at(), "src/ui/view.rs");
    let tree = t.app.tree.as_ref().unwrap();
    assert_eq!(
        tree.collapsed,
        HashSet::from(["tests/".to_string()]),
        "folders without a match stay closed"
    );
    assert!(t.shows("view.rs"), "{}", t.screen());
    // Typing past an earlier match doesn't leave its folders open.
    t.closed(&["src/", "src/ui/", "tests/"]);
    t.app.tree.as_mut().unwrap().select("README.md");
    t.keys("/").typed("m");
    assert_eq!(t.tree_at(), "src/main.rs");
    t.typed("c");
    assert_eq!(t.tree_at(), "tests/mcp.rs");
    let tree = t.app.tree.as_ref().unwrap();
    assert_eq!(
        tree.collapsed,
        HashSet::from(["src/".to_string(), "src/ui/".to_string()])
    );
    t.keys("esc");
    assert_eq!(
        t.app.tree.as_ref().unwrap().collapsed.len(),
        3,
        "esc closes them again"
    );
}

#[test]
fn n_and_shift_n_step_through_tree_matches_and_wrap() {
    let mut t = nested_tree();
    t.keys("/").typed("\\.rs$").keys("ret");
    assert_eq!(t.tree_at(), "src/main.rs");
    t.keys("n");
    assert_eq!(t.tree_at(), "src/ui/mod.rs");
    t.keys("n n");
    assert_eq!(t.tree_at(), "tests/mcp.rs");
    t.keys("n");
    assert_eq!(t.tree_at(), "src/main.rs", "wraps to the top");
    t.keys("N");
    assert_eq!(t.tree_at(), "tests/mcp.rs", "and back to the bottom");
}

#[test]
fn tree_search_backwards_and_without_a_match() {
    let mut t = nested_tree();
    t.keys("?").typed("rs");
    assert_eq!(
        t.tree_at(),
        "tests/mcp.rs",
        "backwards from the top wraps to the end"
    );
    t.keys("esc");
    t.keys("/").typed("nothing");
    assert_eq!(t.tree_at(), "README.md");
    assert_eq!(t.status(), Some("No matches"));
    t.keys("ret");
    assert_eq!(t.tree_at(), "README.md");
}

#[test]
fn a_search_in_the_code_repeats_in_the_tree() {
    let mut t = nested_tree();
    t.keys("esc / v i e w ret");
    assert_eq!(t.app.last_search.as_deref(), Some("view"));
    t.keys("space e n");
    assert_eq!(t.tree_at(), "src/ui/view.rs");
    // And the other way round: a tree search is the code's next search.
    t.keys("/").typed("x").keys("ret");
    assert_eq!(t.app.last_search.as_deref(), Some("x"));
}

#[test]
fn file_tree() {
    let mut t = T::folders(|o, n| {
        std::fs::create_dir_all(n.join("src")).unwrap();
        std::fs::create_dir_all(o.join("src")).unwrap();
        std::fs::write(o.join("src/a.rs"), "1\n2\n").unwrap();
        std::fs::write(n.join("src/a.rs"), "1\nTWO\n3\n").unwrap();
        std::fs::write(n.join("z.md"), "new\n").unwrap();
    });
    t.keys("space e");
    insta::assert_snapshot!(t.screen());
    assert!(t.shows("+2 -1"));
    t.keys("j ret");
    assert_eq!(t.path(), "z.md");
    // Unfocused, keys go to the file again; the tree stays.
    t.keys("g g");
    assert!(t.app.tree.is_some());
    t.keys("space e k ret");
    assert_eq!(t.path(), "src/a.rs");
    // A click opens a file too.
    let y = t.screen().lines().position(|l| l.contains("z.md")).unwrap() as u16;
    t.mouse(MouseEventKind::Down(MouseButton::Left), 3, y);
    assert_eq!(t.path(), "z.md");
    assert!(
        t.app.tree.as_ref().unwrap().focused,
        "a click focuses the tree"
    );
    t.keys("q");
    assert!(t.app.tree.is_none());
    assert!(!t.shows("Changed files"));
}

#[test]
fn view_and_scope_help_list_what_applies() {
    let mut t = T::files(OLD, NEW);
    t.keys("space v");
    assert!(!t.shows("The whole file") && t.shows("Unified") && t.shows("Side by side"));
    t.keys("esc space o");
    assert!(!t.shows("One commit at a time"), "not git");
    assert!(!t.shows("Since your last review"), "nothing submitted");
    assert!(t.shows("Filter files"));
    t.keys("esc");
    let (_, items) = completions("view ");
    assert_eq!(items.len(), 3);
}

#[test]
fn a_revision_against_your_files() {
    let mut t = git_repo_with(|r| Source::revs(r, "HEAD~1", None).unwrap());
    // Since HEAD~1: b.rs (the last commit) and c.rs (uncommitted); not a.rs.
    let paths: Vec<&str> = t
        .app
        .state
        .source
        .changes
        .iter()
        .map(|c| c.path.as_str())
        .collect();
    assert_eq!(paths, ["b.rs", "c.rs"]);
    assert!(t.shows("HEAD~1 vs your files"), "{}", t.screen());
    // The new side is your files: comments work, on the branch's review.
    t.keys("space c").typed("ok?").keys("esc ret");
    assert_eq!(t.bodies(), ["ok?"]);
    assert_eq!(t.app.review.kind, "git");
    assert_eq!(
        t.app.review.base.as_deref(),
        Some("main"),
        "the review keeps its base"
    );
    // Reading the files again keeps comparing with HEAD~1.
    t.app.files_changed();
    assert_eq!(t.app.state.source.changes.len(), 2);
}

#[test]
fn comments_on_two_revisions_keep_their_diff() {
    let mut t = git_repo_with(|r| Source::revs(r, "HEAD~2", Some("HEAD~1")).unwrap());
    let paths: Vec<&str> = t
        .app
        .state
        .source
        .changes
        .iter()
        .map(|c| c.path.as_str())
        .collect();
    assert_eq!(paths, ["a.rs"]);
    assert!(t.shows("HEAD~2 vs HEAD~1"), "{}", t.screen());
    // The cursor opens on the change, `line two`, which is the commit's content.
    t.keys("space c").typed("why two?").keys("esc ret");
    let th = &t.app.state.threads[0];
    let (old, new) = (
        t.app.state.source.old_commit.unwrap(),
        t.app.state.source.new_commit.unwrap(),
    );
    assert_eq!(th.on.old_rev, Some(old.to_string()));
    assert_eq!(th.on.new_rev, Some(new.to_string()));
    let (o, n) = (&old.to_string()[..7], &new.to_string()[..7]);
    assert_eq!(
        th.on.diff.as_deref(),
        Some(format!("HEAD~2 ({o}) vs HEAD~1 ({n})").as_str())
    );
    assert!(t.shows("▸ you: why two?"), "{}", t.screen());
    // Still history after a reload, so the thread keeps its place.
    t.app.files_changed();
    assert_eq!(t.app.state.threads[0].anchor.as_ref().unwrap().start, 2);
    // In the branch's review of your files the same code is still there: shown, marked.
    let repo = t._dir.path().join("repo");
    let theme = Theme::load("default", ColorSupport::TrueColor).unwrap();
    let app = App::new(
        Source::git(&repo, None).unwrap(),
        &config(),
        theme,
        t.agent(),
    )
    .unwrap();
    let mut b = T {
        app,
        term: Terminal::new(TestBackend::new(72, 14)).unwrap(),
        _dir: tempfile::tempdir().unwrap(),
    };
    b.app.open(Target::Path("a.rs".into()));
    b.draw();
    let marker = format!("▸ you: why two? · unsent · on {n}");
    assert!(b.shows(&marker), "{}", b.screen());
}

#[test]
fn comments_on_one_commit_stay_on_it() {
    let mut t = git_repo();
    t.keys("space o c");
    assert_eq!(t.path(), "a.rs");
    t.keys("space c").typed("on the commit").keys("esc ret");
    let th = t.app.state.threads[0].clone();
    let commit = &t.app.commits[0];
    assert_eq!(th.on.new_rev, Some(commit.id.to_string()));
    let diff = th.on.diff.unwrap();
    assert!(
        diff.starts_with("commit 1/2 ") && diff.contains("\"two\" vs its parent"),
        "{diff}"
    );
    // The next commit doesn't touch that code; back on your files, it shows where the code is.
    t.keys("] C");
    assert!(!t.shows("on the commit"));
    t.keys("space o b");
    t.app.open(Target::Path("a.rs".into()));
    t.draw();
    assert!(t.shows("▸ you: on the commit"), "{}", t.screen());
    // A comment made on your files shows in the commit's view too, marked as such.
    t.keys("space c").typed("on files").keys("esc ret");
    t.keys("space o c");
    assert!(
        t.shows("on files · unsent · on your files"),
        "{}",
        t.screen()
    );
}

#[test]
fn change_motions_work_in_every_view() {
    let old = lines(30);
    let new = old
        .replace("line 5\n", "line five\n")
        .replace("line 20\n", "line twenty\nline 20.5\n");
    // The same lines in every view, as the file view (helix-core's motions) finds them.
    let walk = |view: &str| {
        let mut t = T::files(&old, &new);
        t.keys(&format!("space v {view} g g"));
        let mut seen = vec![];
        for keys in ["] g", "] g", "] g", "[ g", "] G", "[ G"] {
            t.keys(keys);
            seen.push(t.app.cursor_new_line());
        }
        seen
    };
    let file = walk("f");
    assert_eq!(file[0], 4, "{file:?}");
    assert_ne!(file[0], file[1], "two changes: {file:?}");
    assert_eq!(walk("u"), file, "unified");
    assert_eq!(walk("s"), file, "side by side");
}

#[test]
fn the_file_tree_starts_open() {
    let dir = tempfile::tempdir().unwrap();
    let (o, n) = (dir.path().join("old.rs"), dir.path().join("main.rs"));
    std::fs::write(&o, OLD).unwrap();
    std::fs::write(&n, NEW).unwrap();
    let mut t = T::new(
        Source::diff(&o, &n).unwrap(),
        dir,
        72,
        14,
        &Config::default(),
    );
    assert!(t.shows("Changed files"), "{}", t.screen());
    assert!(!t.app.tree.as_ref().unwrap().focused, "keys go to the file");
    // So `j` moves the cursor in the file, not in the tree.
    let line = t.app.cursor_new_line();
    t.keys("j");
    assert_eq!(t.app.cursor_new_line(), line + 1);
    t.keys("space e q");
    assert!(t.app.tree.is_none());
}

// ---- The palette: C-g anywhere, space ? in normal mode ----

impl T {
    /// The palette's title (its second column's header) and its rows' text.
    fn palette(&self) -> (String, Vec<String>) {
        let p = self.app.pickers.last().expect("the palette is open");
        let screen = self.screen();
        (
            p.columns[1].to_string(),
            screen.lines().map(str::to_string).collect(),
        )
    }
}

#[test]
fn palette_screen() {
    let mut t = T::files_sized(OLD, NEW, 100, 20);
    t.keys("C-g");
    insta::assert_snapshot!(t.screen());
}

#[test]
fn palette_lists_what_applies_and_runs_it() {
    let mut t = T::files(OLD, NEW);
    t.keys("C-g");
    let (title, _) = t.palette();
    assert_eq!(title, "Normal mode");
    // Typing filters by what a key does.
    t.typed("comment");
    assert!(
        t.shows("space c") && t.shows("Comment on the selected lines"),
        "{}",
        t.screen()
    );
    // Nothing unsent and no thread: those actions aren't listed.
    assert!(!t.shows("Reply to the thread"));
    assert!(!t.shows("Send comments without a verdict"));
    // ret runs the command, here a submit.
    t.keys("esc C-g");
    t.typed("approve").keys("ret");
    assert!(t.app.pickers.is_empty());
    assert!(
        t.said().starts_with("Round 1 submitted: approved"),
        "{}",
        t.said()
    );
    // `space ?` is the same palette, and C-g again closes it.
    t.keys("space ?");
    assert_eq!(t.palette().0, "Normal mode");
    t.keys("C-g");
    assert!(t.app.pickers.is_empty());
    // A command that waits for a character still does.
    t.keys("C-g")
        .typed("Move to next occurrence of char")
        .keys("ret");
    t.keys("x");
    assert!(t.app.next_key.is_none());
}

#[test]
fn palette_in_the_comment_box() {
    let mut t = T::files(OLD, NEW);
    t.keys("space c").typed("draft").keys("C-g");
    assert_eq!(t.palette().0, "Comment box: insert mode");
    assert!(t.shows("New line"));
    // esc goes back to typing, the text kept.
    t.keys("esc");
    assert!(t.app.pickers.is_empty());
    assert!(t.app.comment_box.as_ref().unwrap().insert);
    t.typed("!");
    assert_eq!(t.app.comment_box.as_ref().unwrap().body(), "draft!");
    // In normal mode, the box's own keys first; what waits for the box isn't listed.
    t.keys("esc C-g");
    assert_eq!(t.palette().0, "Comment box: normal mode");
    assert!(t.shows("Save the comment") && t.shows("Close the box"));
    assert!(
        !t.shows("Goto next changed file"),
        "gn waits until the box closes"
    );
    assert!(!t.shows("Replay macro"), "q closes the box here");
}

#[test]
fn palette_in_the_tree_picker_and_prompt() {
    // Tall enough to list every tree key.
    let mut t = T::files_sized(OLD, NEW, 100, 30);
    // The file tree.
    t.app.app_command(AppAction::FileTree);
    t.keys("C-g");
    assert_eq!(t.palette().0, "File tree");
    assert!(t.shows("Close the tree"));
    assert!(
        t.shows("Open the file, or open or close the folder"),
        "{}",
        t.screen()
    );
    assert!(t.shows("Close the folder, or go to the one it's in"));
    assert!(t.shows("Search the names"));
    // Choosing a key runs its command in the tree, which keeps focus.
    t.keys("esc C-g")
        .typed("search the names backwards")
        .keys("ret");
    assert!(t.app.pickers.is_empty() && t.app.tree.as_ref().unwrap().focused);
    assert!(t.shows("search:"), "{}", t.screen());
    t.keys("esc C-g").typed("close the tree");
    assert!(t.shows("Close the tree"), "{}", t.screen());
    t.keys("ret");
    assert!(t.app.tree.is_none(), "the palette closed the tree");
    // A picker: the palette covers it, and esc brings it back with its query.
    t.keys("space g").typed("main").keys("C-g");
    assert_eq!(t.palette().0, "Picker");
    assert_eq!(t.app.pickers.len(), 2);
    t.keys("C-g"); // C-g again goes back too
    assert_eq!(t.app.pickers.len(), 1);
    t.keys("C-g ret"); // so does choosing a key it only lists
    assert_eq!(t.app.pickers.len(), 1);
    assert!(!t.app.pickers[0].palette);
    t.keys("C-g esc");
    assert!(t.shows("main.rs") && t.app.pickers.len() == 1);
    t.keys("esc");
    // The command line: the line is kept.
    t.keys(":").typed("subm").keys("C-g");
    assert_eq!(t.palette().0, "Command line");
    assert!(t.shows("Complete"));
    t.keys("esc");
    assert_eq!(t.app.prompt.as_ref().unwrap().line, "subm");
}

#[test]
fn the_dashboard_lists_reviews_and_opens_another() {
    let mut t = T::files_sized(OLD, NEW, 220, 16);
    // A second review in the same store, made as another glos would.
    let dir = t._dir.path().to_path_buf();
    let (o, n) = (dir.join("other.old"), dir.join("other.rs"));
    std::fs::write(&o, "a\n").unwrap();
    std::fs::write(&n, "b\n").unwrap();
    let other = Review::open(&t.app.conn, &Source::diff(&o, &n).unwrap()).unwrap();

    t.keys("space l");
    assert!(
        t.shows("state") && t.shows("unsent") && t.shows("outdated"),
        "{}",
        t.screen()
    );
    assert!(t.shows("other.rs") && t.shows("main.rs"), "{}", t.screen());
    assert!(t.shows("2/2"));

    // It refreshes on its own when another process writes.
    other.request(&t.app.conn, "claude", "", &[]).unwrap();
    t.app.data_version -= 1; // as if another process wrote
    t.app.refresh();
    t.draw();
    assert!(t.shows("waiting for review"), "{}", t.screen());

    // Its own review: the dashboard just closes.
    t.typed("main.rs").keys("ret");
    assert!(t.app.pickers.is_empty() && !t.app.quit && t.app.switch.is_none());

    // Another: glos closes this review and opens that one.
    t.keys("space l").typed("other").keys("ret");
    assert!(t.app.quit);
    let s = t.app.switch.as_ref().expect("a review to open");
    assert_eq!(s.changes[0].path, "other.rs");
}

#[test]
fn the_dashboard_is_a_command_and_a_key() {
    let mut t = T::files(OLD, NEW);
    t.keys(": d a s h b o a r d ret");
    assert!(t.app.pickers.last().is_some_and(dashboard::is_dashboard));
    t.keys("esc");
    assert!(t.app.pickers.is_empty());
    // C-g over it lists the picker's keys.
    t.keys("space l C-g");
    assert!(t.shows("Filter the list"), "{}", t.screen());
    t.keys("C-g");
    assert!(t.app.pickers.last().is_some_and(dashboard::is_dashboard));
}

#[test]
fn close_needs_everything_sent_then_quits() {
    let mut t = T::files(OLD, NEW);
    t.keys("x space c").typed("hmm").keys("esc ret");
    t.keys(": c l o s e ret");
    assert!(!t.app.quit);
    assert!(t.said().contains("1 comment not sent"), "{}", t.said());
    t.keys(": s u b m i t space c o m m e n t ret");
    t.keys(": c l o s e ret");
    assert!(t.app.quit);
    assert_eq!(t.app.review.reload(&t.app.conn).unwrap().status, "closed");
}

// ---- Folding ----

/// Forty lines with line 20 changed.
fn folded() -> T {
    let old = lines(40);
    T::files(&old, &old.replace("line 20\n", "line twenty\n"))
}

fn text(t: &T) -> String {
    t.app.file().text.to_string()
}

/// The screen row showing `s`.
fn row_of(t: &T, s: &str) -> usize {
    t.screen()
        .lines()
        .position(|l| l.contains(s))
        .unwrap_or_else(|| panic!("{s} not shown:\n{}", t.screen()))
}

const FOLDED: &str = "↕ lines 1–16 (16)  ret opens\nline 17\nline 18\nline 19\nline twenty\n\
                      line 21\nline 22\nline 23\n↕ lines 24–40 (17)  ret opens\n";

#[test]
fn folded_file_view() {
    insta::assert_snapshot!(folded().screen());
}

#[test]
fn a_review_opens_folded_on_its_changes() {
    let t = folded();
    assert_eq!(text(&t), FOLDED);
    assert!(t.sel().contains("#[l|]#ine twenty"), "{}", t.sel());
    // The gutter numbers lines as the file does, and marks the change.
    assert!(t.shows("   17  line 17"), "{}", t.screen());
    assert!(t.shows("   20 ▍line twenty"), "{}", t.screen());
    // A file with nothing far from its changes isn't folded at all.
    let t = T::files(OLD, NEW);
    assert!(t.app.file().rows.is_none());
    // Nor is one opened for context.
    let mut t = T::folders(|o, n| {
        std::fs::write(o.join("a"), "x\n").unwrap();
        std::fs::write(n.join("a"), "y\n").unwrap();
        std::fs::write(n.join("same"), lines(40)).unwrap();
        std::fs::write(o.join("same"), lines(40)).unwrap();
    });
    t.keys("space f").typed("same").keys("ret");
    assert_eq!(t.path(), "context:same");
    assert_eq!(text(&t), lines(40));
}

#[test]
fn opening_and_closing_folds() {
    let mut t = folded();
    // zo on the first fold opens it, the cursor on its first line.
    t.keys("g g z o");
    assert!(text(&t).starts_with("line 1\nline 2\n"), "{}", text(&t));
    assert!(t.sel().starts_with("#[l|]#ine 1"), "{}", t.sel());
    // za closes it again, from any of its lines.
    t.keys("j j z a");
    assert_eq!(text(&t), FOLDED);
    assert!(t.sel().starts_with("#[↕|]#"), "{}", t.sel());
    t.keys("z a");
    assert!(text(&t).starts_with("line 1\n"));
    t.keys("z a");
    // za where nothing folds says so.
    t.keys("] g z a");
    assert_eq!(t.said(), "No fold here");
    // zR shows the whole file; zM folds everything again.
    t.keys("z R");
    assert!(t.app.file().rows.is_none(), "the file itself");
    assert_eq!(text(&t), lines(40).replace("line 20\n", "line twenty\n"));
    assert_eq!(t.app.cursor_new_line(), 19, "the cursor stays");
    t.keys("z M");
    assert_eq!(text(&t), FOLDED);
    // zo off a fold.
    t.keys("z o");
    assert_eq!(t.said(), "No fold here");
    // Sticky Z works too.
    t.keys("Z R esc");
    assert!(t.app.file().rows.is_none());
}

#[test]
fn ret_and_a_click_open_a_fold() {
    let mut t = folded();
    t.keys("g g ret");
    assert!(text(&t).starts_with("line 1\n"));
    t.keys("z M");
    let y = row_of(&t, "↕ lines 24–40") as u16;
    t.mouse(MouseEventKind::Down(MouseButton::Left), 10, y);
    assert!(text(&t).ends_with("line 39\nline 40\n"), "{}", text(&t));
    assert!(t.shows("line 24"));
}

#[test]
fn context_lines_and_the_cursor_staying_put() {
    let old = lines(40);
    let mut t = T::files_sized(&old, &old.replace("line 20\n", "line twenty\n"), 72, 30);
    t.keys("3 j");
    let row = row_of(&t, "line 23");
    t.keys("z +");
    assert_eq!(t.said(), "4 lines of context");
    assert!(text(&t).starts_with("↕ lines 1–15 (15)  ret opens\nline 16\n"));
    assert_eq!(row_of(&t, "line 23"), row, "the line stays where it was");
    assert!(t.sel().contains("#[l|]#ine 23"));
    t.keys("3 k");
    t.keys("z minus z minus z minus z minus");
    assert_eq!(t.said(), "0 lines of context");
    assert_eq!(
        text(&t),
        "↕ lines 1–19 (19)  ret opens\nline twenty\n↕ lines 21–40 (20)  ret opens\n"
    );
    t.keys("z minus");
    assert_eq!(t.said(), "0 lines of context", "never below 0");
    // An opened fold stays open as the context changes.
    t.keys("g g z o z +");
    assert!(text(&t).starts_with("line 1\n"), "{}", text(&t));
    // The context applies to every file and view.
    t.keys("space v u");
    assert!(t.shows("line twenty"));
    assert!(t.shows("line 19") && !t.shows("line 18"), "{}", t.screen());
}

#[test]
fn folds_you_make_and_remove() {
    let mut t = folded();
    // x x: line twenty and 21, folded with what they hide said.
    t.keys("x x z f");
    assert!(t.shows("↕ lines 20–21 (2) · 1 changed"), "{}", t.screen());
    assert!(!t.shows("line twenty"));
    assert!(t.sel().contains("#[↕|]# lines 20–21"), "{}", t.sel());
    // zd removes it.
    t.keys("z d");
    assert!(t.shows("line twenty"));
    t.keys("z d");
    assert_eq!(t.said(), "No fold you made here");
    // Several selections make several folds.
    t.keys("g g z o g g x C C z f");
    // C copies the selection down: three selections, on lines 1, 2 and 3, each a fold, shown as
    // one stretch.
    assert!(
        text(&t).starts_with("↕ lines 1–3 (3)  ret opens\nline 4\n"),
        "{}",
        text(&t)
    );
    // zR opens what you made, and zM closes it again.
    t.keys("z R");
    assert!(text(&t).starts_with("line 1\n"));
    t.keys("z M");
    assert!(
        text(&t).starts_with("↕ lines 1–3 (3)  ret opens\n↕ lines 4–16 (13)  ret opens\nline 17\n"),
        "yours stays apart from the context's: {}",
        text(&t)
    );
}

#[test]
fn jumps_open_the_folds_they_land_in() {
    let mut t = folded();
    // ]g opens a fold you made over the change, on the change.
    t.keys("x z f g g ] g");
    assert!(t.sel().contains("#[l|]#ine twenty"), "{}", t.sel());
    assert!(!t.shows("1 changed"));
    // Threads are never folded by the context.
    t.keys("z R : 3 0 ret space c")
        .typed("thirty")
        .keys("esc ret z M");
    assert!(t.shows("line 30"), "{}", t.screen());
    assert!(t.shows("▸ you: thirty"));
    // A fold you make over one hides it, says so, and ]d opens it again.
    t.keys("x z f");
    assert!(t.shows("↕ line 30 (1) · 1 thread"), "{}", t.screen());
    assert!(!t.shows("▸ you: thirty"));
    t.keys("g g ] d");
    assert!(t.shows("▸ you: thirty"), "{}", t.screen());
    assert!(t.sel().contains("#[line 30\n|]#"), "{}", t.sel());
    // space n opens a folded line an agent suggested.
    t.app
        .review
        .suggest(&t.agent(), "claude", "main.rs", 5, "look")
        .unwrap();
    t.app.refresh();
    t.keys("space n");
    assert!(t.sel().contains("#[line 5\n|]#"), "{}", t.sel());
}

#[test]
fn folds_last_until_the_file_changes() {
    let mut t = T::folders(|o, n| {
        for f in ["a.txt", "b.txt"] {
            std::fs::write(o.join(f), lines(40)).unwrap();
            std::fs::write(n.join(f), lines(40).replace("line 20\n", "x\n")).unwrap();
        }
    });
    t.keys("g g z o");
    assert!(text(&t).starts_with("line 1\n"));
    // Another file and back: still open.
    t.keys("g n");
    assert!(text(&t).starts_with("↕ lines 1–16"));
    t.keys("g p");
    assert!(text(&t).starts_with("line 1\n"), "{}", text(&t));
    // The file changes: folded afresh.
    let new = t._dir.path().join("new/a.txt");
    std::fs::write(&new, lines(40).replace("line 20\n", "y\n")).unwrap();
    t.app.files_changed();
    t.draw();
    assert!(text(&t).starts_with("↕ lines 1–16"), "{}", text(&t));
}

#[test]
fn unified_and_side_by_side_fold_with_the_same_keys() {
    let mut t = folded();
    t.keys("space v u");
    assert!(t.shows("↕ lines 1–16 (16)"));
    t.keys("z R");
    assert!(text(&t).starts_with("line 1\n"));
    assert!(text(&t).contains("line 20\nline twenty\n"));
    t.keys("z M space v s");
    assert!(t.shows("↕ lines 1–16 (16)"));
    t.keys("g g z o");
    let left = t.app.split_left().unwrap().text.to_string();
    assert!(left.starts_with("line 1\n"), "both sides open: {left}");
    assert_eq!(left.lines().count(), text(&t).lines().count());
    // Each view keeps its own folds.
    t.keys("space v u");
    assert!(text(&t).starts_with("↕ lines 1–16"));
    // A comment next to a fold goes on the right line.
    t.keys("space v f g g j x space c");
    assert!(t.shows("Comment on main.rs:17 "), "{}", t.screen());
}

const FNS_OLD: &str = "fn a() {\n    1\n}\n\nfn b() {\n    2\n}\n";
const FNS_NEW: &str = "fn a() {\n    1\n}\n\nfn b() {\n    two\n}\n";

#[test]
fn tree_sitter_objects_and_motions() {
    let mut t = T::files(FNS_OLD, FNS_NEW);
    t.keys("z R g g m a f");
    assert!(
        t.sel().starts_with("#[fn a() {\n    1\n}|]#"),
        "{}",
        t.sel()
    );
    t.keys("m i f");
    // Rust's function.inside is the body.
    assert!(
        t.sel().starts_with("fn a() #[{\n    1\n}|]#"),
        "{}",
        t.sel()
    );
    t.keys("g g ] f");
    assert!(t.sel().contains("#[fn b() {\n    two\n}|]#"), "{}", t.sel());
    t.keys("[ f");
    assert!(t.sel().contains("#[|fn a() {\n    1\n}]#"), "{}", t.sel());
    // In the unified view, objects come from the new side and select its rows.
    t.keys("space v u g g ] f");
    assert!(
        t.sel().contains("#[fn b() {\n    2\n    two\n}|]#"),
        "{}",
        t.sel()
    );
    // Folding a function: select it, then zf.
    t.keys("z f");
    assert!(t.shows("↕ lines 5–7 (4) · 2 changed"), "{}", t.screen());
    // Plain text has no syntax tree.
    let mut t = T::folders(|o, n| {
        std::fs::write(o.join("a.txt"), "x\n").unwrap();
        std::fs::write(n.join("a.txt"), "y\n").unwrap();
    });
    t.keys("m a f");
    assert!(
        t.said().contains("Syntax-tree is not available"),
        "{}",
        t.said()
    );
}

#[test]
fn a_text_object_opens_the_folds_it_covers() {
    let body: String = (1..=20).map(|i| format!("    let x{i} = {i};\n")).collect();
    let old = format!("fn a() {{\n{body}}}\n");
    let new = old.replace("x1 = 1;", "x1 = 100;");
    let mut t = T::files_sized(&old, &new, 72, 40);
    assert!(t.shows("↕ lines 6–22 (17)"), "{}", t.screen());
    t.keys("m a f");
    assert!(!t.shows("↕"), "the whole function is shown: {}", t.screen());
    assert!(t.sel().starts_with("#[fn a() {"), "{}", t.sel());
    assert!(t.sel().ends_with("}|]#\n"), "{}", t.sel());
    t.keys("z f");
    assert_eq!(text(&t), "↕ lines 1–22 (22) · 1 changed  ret opens\n");
    // The status bar gives the line a fold starts on.
    assert!(
        t.screen().lines().any(|l| l.ends_with("sel  1:1")),
        "{}",
        t.screen()
    );
}

#[test]
fn a_fold_is_a_band_across_the_view() {
    let t = folded();
    let (band, hint) = diffview::fold_styles(&t.app.theme);
    assert!(
        band.bg.is_some(),
        "the default theme gives the band a background"
    );
    let y = row_of(&t, "↕ lines 1–16") as u16;
    let w = t.term.backend().buffer().area.width;
    // Gutter to right edge.
    for x in [0, 3, 20, w - 1] {
        assert_eq!(t.cell(x, y).bg, band.bg.unwrap(), "column {x}");
    }
    // Other lines aren't banded; the key hint is in its own, dimmer colour.
    let line = row_of(&t, "line 17") as u16;
    assert_ne!(t.cell(w - 1, line).bg, band.bg.unwrap());
    let hint_x = t
        .screen()
        .lines()
        .nth(y as usize)
        .unwrap()
        .find("ret opens")
        .unwrap();
    let hint_x = t.screen().lines().nth(y as usize).unwrap()[..hint_x]
        .chars()
        .count() as u16;
    assert_eq!(Some(t.cell(hint_x, y).fg), hint.fg);
    assert_ne!(hint.fg, band.fg);
    // A theme's own `ui.virtual.fold` wins.
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("mine.toml"),
        "inherits = \"default\"\n\"ui.virtual.fold\" = { fg = \"#112233\", bg = \"#445566\" }\n",
    )
    .unwrap();
    let mine = Theme::load_from("mine", ColorSupport::TrueColor, Some(dir.path())).unwrap();
    let (band, _) = diffview::fold_styles(&mine);
    assert_eq!(band.bg, Some(Color::Rgb(0x44, 0x55, 0x66)));
    assert_eq!(band.fg, Some(Color::Rgb(0x11, 0x22, 0x33)));
}

mod mouse;
mod perf;
