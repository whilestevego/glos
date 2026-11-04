//! M0 spike 4: drive helix-core motions and selections over a ropey buffer.
//! Wrappers below mirror helix-term/src/commands.rs at 25.07.1 (MPL-2.0).
use helix_core::{
    match_brackets, movement,
    test::{plain, print},
    textobject::{self, TextObject},
    Range, Rope, Selection,
};

/// Apply one normal-mode key to (text, selection). `count` as in Helix.
fn apply(text: &Rope, sel: Selection, key: &str, count: usize, select_mode: bool) -> Selection {
    let s = text.slice(..);
    match key {
        // move_word_impl / extend_word_impl
        "w" if select_mode => sel.transform(|r| {
            let pos = movement::move_next_word_start(s, r, count).cursor(s);
            r.put_cursor(s, pos, true)
        }),
        "w" => sel.transform(|r| movement::move_next_word_start(s, r, count)),
        "b" => sel.transform(|r| movement::move_prev_word_start(s, r, count)),
        "e" => sel.transform(|r| movement::move_next_word_end(s, r, count)),
        // extend_line_below (Helix's `x`)
        "x" => sel.transform(|r| {
            let (sl, el) = r.line_range(s);
            let start = text.line_to_char(sl);
            let end = text.line_to_char((el + 1).min(text.len_lines()));
            let n = if r.from() == start && r.to() == end { count } else { count - 1 };
            Range::new(start, text.line_to_char((el + 1 + n).min(text.len_lines())))
        }),
        "%" => Selection::single(0, text.len_chars()),
        "mm" => sel.transform(|r| {
            let pos = r.cursor(s);
            match match_brackets::find_matching_bracket_plaintext(s, pos) {
                Some(m) => r.put_cursor(s, m, select_mode),
                None => r,
            }
        }),
        "miw" => sel.transform(|r| textobject::textobject_word(s, r, TextObject::Inside, count, false)),
        "mi(" => sel.transform(|r| {
            textobject::textobject_pair_surround(None, s, r, TextObject::Inside, '(', count)
        }),
        "ma(" => sel.transform(|r| {
            textobject::textobject_pair_surround(None, s, r, TextObject::Around, '(', count)
        }),
        _ => panic!("unmapped key {key}"),
    }
}

/// Helix-notation round trip: `#[h|]#ello` in, keys applied, notation out.
fn run(input: &str, keys: &[(&str, usize)], select_mode: bool) -> String {
    let (text, mut sel) = print(input);
    let rope = Rope::from(text);
    for (k, n) in keys {
        // As helix-view's Document::set_selection: keeps cursors 1-grapheme wide.
        sel = apply(&rope, sel, k, *n, select_mode).ensure_invariants(rope.slice(..));
    }
    plain(rope, &sel)
}

fn main() {
    let out = run("#[h|]#ello world foo\nnext line\n", &[("w", 2), ("x", 1)], false);
    println!("{out}");
}

#[cfg(test)]
mod tests {
    use super::run;

    #[test]
    fn word_motions() {
        assert_eq!(run("#[h|]#ello world", &[("w", 1)], false), "#[hello |]#world");
        assert_eq!(run("#[h|]#ello world foo", &[("w", 2)], false), "hello #[world |]#foo");
        assert_eq!(run("#[h|]#ello world", &[("e", 1)], false), "#[hello|]# world");
        assert_eq!(run("hello wor#[l|]#d", &[("b", 1)], false), "hello #[|worl]#d");
    }

    #[test]
    fn select_mode_extends() {
        assert_eq!(run("#[h|]#ello world foo", &[("w", 1), ("w", 1)], true), "#[hello world |]#foo");
    }

    #[test]
    fn line_select_x() {
        let t = "a#[b|]#c\ndef\nghi\n";
        assert_eq!(run(t, &[("x", 1)], false), "#[abc\n|]#def\nghi\n");
        assert_eq!(run(t, &[("x", 1), ("x", 1)], false), "#[abc\ndef\n|]#ghi\n");
        assert_eq!(run(t, &[("x", 2)], false), "#[abc\ndef\n|]#ghi\n");
    }

    #[test]
    fn select_all_and_brackets() {
        assert_eq!(run("a#[b|]#c", &[("%", 1)], false), "#[abc|]#");
        assert_eq!(run("f#[(|]#a, b)", &[("mm", 1)], false), "f(a, b#[)|]#");
    }

    #[test]
    fn textobjects() {
        assert_eq!(run("foo(a, #[b|]#)", &[("mi(", 1)], false), "foo(#[a, b|]#)");
        assert_eq!(run("foo(a, #[b|]#)", &[("ma(", 1)], false), "foo#[(a, b)|]#");
        assert_eq!(run("one tw#[o|]# three", &[("miw", 1)], false), "one #[two|]# three");
        // count: mi2( selects the outer pair's contents
        assert_eq!(run("f(g(#[x|]#))", &[("mi(", 2)], false), "f(#[g(x)|]#)");
    }

    #[test]
    fn multiple_cursors() {
        let t = "#[a|]#a bb\n#(c|)#c dd\n";
        assert_eq!(run(t, &[("w", 1)], false), "#[aa |]#bb\n#(cc |)#dd\n");
    }
}
