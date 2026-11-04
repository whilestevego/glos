//! The app driven as a user drives it: keys and mouse events in, then the selection (in Helix's
//! `#[h|]#ello` notation), the view and the screen checked. Screen snapshots lock the parts that
//! must look like Helix.

use std::path::Path;

use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use helix_core::test::plain;
use ratatui::{Terminal, backend::TestBackend, style::Color};
use tempfile::TempDir;

use super::*;

/// An app over a review, with a test terminal of a fixed size.
struct T {
    app: App,
    term: Terminal<TestBackend>,
    _dir: TempDir,
}

const OLD: &str = "fn main() {\n    println!(\"hi\");\n}\n\nfn gone() {}\n";
const NEW: &str = "fn main() {\n    println!(\"hello\");\n    let x = 1;\n}\n";

fn config() -> Config {
    Config::default()
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
        let app = App::new(source, config, theme).unwrap();
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
        match &self.app.cur {
            Cur::Change(i) => self.app.source.changes[*i].path.clone(),
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
    for _ in 0..t.app.files.len() {
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
    assert!(t.app.mode == Mode::Select);
    assert!(t.keys("esc").app.mode == Mode::Normal);
    assert!(t.keys("v v").app.mode == Mode::Normal);
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
    assert!(t.app.picker.is_none());
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
    assert!(t.app.wrap);
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
    assert!(t.app.mode == Mode::Normal, "yanking leaves select mode");
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
    assert!(t.app.wrap);
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
    let (list, _) = Picker::areas(t.app.screen);
    t.mouse(
        MouseEventKind::Down(MouseButton::Left),
        list.x + 4,
        list.y + 4,
    );
    assert!(t.app.picker.is_none());
    assert_eq!(t.path(), "b");
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
