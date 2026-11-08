//! Re-anchoring: placing a thread again after its file changed.
//!
//! The old position is mapped through the line diff between the file as it was when the thread
//! was anchored and the file now. If the diff doesn't touch the anchored lines they've only
//! moved. If it does, the anchored text is searched for in the new file, nearest the mapped
//! position first, with the context lines breaking ties. A copy far away, or a copy of a short or
//! common line without its context, doesn't count, so a thread on `}` doesn't jump to another
//! `}`. If no copy counts but the lines the diff maps the thread to are much like its text, the
//! thread stays on them, so fixing a typo doesn't outdate it. Otherwise the thread is outdated.

use crate::{diff, diff_words};

/// How far from where the thread was mapped a copy of its text may be, unless all of its context
/// came with it.
const WINDOW: usize = 200;
/// Anchored text with fewer non-whitespace characters than this is short, and needs a context
/// line to match.
const SHORT: usize = 8;
/// Anchored text that appears more often than this in the new file is common, and needs a context
/// line to match.
const COMMON: usize = 3;
/// How alike the mapped lines must be to the anchored text to keep the thread on them.
const SIMILAR: f32 = 0.7;

/// A thread's anchor as stored: lines `start..=end` (0-based) of `old`, the file's content when
/// it was anchored, and the anchored lines' text with up to three lines of context either side.
pub struct Anchored<'a> {
    pub old: &'a str,
    pub start: usize,
    pub end: usize,
    pub text: &'a str,
    pub above: &'a str,
    pub below: &'a str,
}

/// Lines without their line endings, so a file's last line matches with or without one.
fn lines(s: &str) -> Vec<&str> {
    s.split_inclusive('\n')
        .map(|l| l.trim_end_matches('\n').trim_end_matches('\r'))
        .collect()
}

/// Where the anchored lines are in `new` (0-based, inclusive), or `None` if they're gone.
pub fn place(a: &Anchored, new: &str) -> Option<(usize, usize)> {
    if a.old == new {
        return Some((a.start, a.end));
    }
    // How far lines before the anchor moved, and whether any change reaches into it.
    let mut shift = 0isize;
    let mut touched = false;
    for h in diff::hunks(a.old, new) {
        let (bs, be) = (h.before.start as usize, h.before.end as usize);
        let grew = h.after.len() as isize - h.before.len() as isize;
        // A hunk ending at or before the first anchored line is above it (an insertion right
        // above included); one starting after the last line is below. Anything else is inside.
        if be <= a.start {
            shift += grew;
        } else if bs > a.end {
            break;
        } else {
            touched = true;
            break;
        }
    }
    let mapped = (a.start as isize + shift).max(0) as usize;
    if !touched {
        return Some((mapped, mapped + (a.end - a.start)));
    }

    // The anchored lines changed or moved with other changes: look for their text.
    let want = lines(a.text);
    if want.is_empty() {
        return None;
    }
    let have = lines(new);
    let (above, below) = (lines(a.above), lines(a.below));
    let context = |i: usize| {
        let up = above
            .iter()
            .rev()
            .zip(have[..i].iter().rev())
            .take_while(|(x, y)| x == y)
            .count();
        let down = below
            .iter()
            .zip(&have[i + want.len()..])
            .take_while(|(x, y)| x == y)
            .count();
        up + down
    };
    let copies: Vec<usize> = (0..=have.len().saturating_sub(want.len()))
        .filter(|&i| have.len() >= want.len() && have[i..i + want.len()] == want[..])
        .collect();
    let chars = want
        .iter()
        .flat_map(|l| l.chars())
        .filter(|c| !c.is_whitespace());
    let needs_context = chars.count() < SHORT || copies.len() > COMMON;
    let full = above.len() + below.len();
    let found = copies
        .into_iter()
        .filter(|&i| !needs_context || context(i) > 0)
        .filter(|&i| i.abs_diff(mapped) <= WINDOW || context(i) == full)
        .min_by_key(|&i| (i.abs_diff(mapped), usize::MAX - context(i)))
        .map(|i| (i, i + want.len() - 1));
    found.or_else(|| edited(a, new, &have))
}

/// The lines the diff maps the anchored lines to, if they're much like the anchored text: the
/// same code, edited a little.
fn edited(a: &Anchored, new: &str, have: &[&str]) -> Option<(usize, usize)> {
    let last = have.len().checked_sub(1)?;
    let hunks = diff::hunks(a.old, new);
    let (s, e) = (map_line(&hunks, a.start), map_line(&hunks, a.end));
    let (s, e) = (s.min(last), e.min(last).max(s.min(last)));
    let text = have[s..=e].join("\n");
    (diff_words::text_similarity(a.text, &text) >= SIMILAR).then_some((s, e))
}

/// Where line `line` of the old file is in the new one. A line inside a change goes to the line
/// at the same place in what replaced it, or the line after if it was only removed.
fn map_line(hunks: &[diff::Hunk], line: usize) -> usize {
    let mut shift = 0isize;
    for h in hunks {
        let (bs, be) = (h.before.start as usize, h.before.end as usize);
        if line < bs {
            break;
        }
        if line < be {
            let after = h.after.len();
            return h.after.start as usize + (line - bs).min(after.saturating_sub(1));
        }
        shift += h.after.len() as isize - h.before.len() as isize;
    }
    (line as isize + shift).max(0) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The anchor `start..=end` of `old` with its text and three lines of context, as the store
    /// keeps it.
    fn at(old: &str, start: usize, end: usize) -> (String, String, String) {
        let l: Vec<&str> = old.split_inclusive('\n').collect();
        (
            l[start..=end].concat(),
            l[start.saturating_sub(3)..start].concat(),
            l[(end + 1).min(l.len())..(end + 4).min(l.len())].concat(),
        )
    }

    fn placed(old: &str, start: usize, end: usize, new: &str) -> Option<(usize, usize)> {
        let (text, above, below) = at(old, start, end);
        place(
            &Anchored {
                old,
                start,
                end,
                text: &text,
                above: &above,
                below: &below,
            },
            new,
        )
    }

    const F: &str = "a\nb\nc\nd\ne\nf\ng\n";

    #[test]
    fn unchanged_files_keep_their_place() {
        assert_eq!(placed(F, 2, 3, F), Some((2, 3)));
    }

    #[test]
    fn changes_elsewhere_move_it() {
        // Lines added above shift it down; removed above shift it up.
        assert_eq!(placed(F, 2, 3, "x\ny\na\nb\nc\nd\ne\nf\ng\n"), Some((4, 5)));
        assert_eq!(placed(F, 2, 3, "c\nd\ne\nf\ng\n"), Some((0, 1)));
        // A line changed above doesn't move it.
        assert_eq!(placed(F, 2, 3, "a\nB\nc\nd\ne\nf\ng\n"), Some((2, 3)));
        // Changes below never move it.
        assert_eq!(placed(F, 2, 3, "a\nb\nc\nd\nnew\ne\n"), Some((2, 3)));
        assert_eq!(placed(F, 2, 3, "a\nb\nc\nd\n"), Some((2, 3)));
        // Inserted right above or right below: still the same lines.
        assert_eq!(placed(F, 2, 3, "a\nb\nnew\nc\nd\ne\nf\ng\n"), Some((3, 4)));
        assert_eq!(placed(F, 2, 3, "a\nb\nc\nd\nnew\ne\nf\ng\n"), Some((2, 3)));
        // Several changes above add up.
        assert_eq!(placed(F, 5, 5, "x\na\nc\nd\ny\nz\ne\nf\ng\n"), Some((7, 7)));
    }

    /// Lines long enough to need no context: `let v0 = 0;` to `let v{n-1} = n-1;`.
    fn long(n: usize) -> Vec<String> {
        (0..n).map(|i| format!("let v{i} = {i};")).collect()
    }

    /// Lines joined into a file.
    fn file(lines: &[String]) -> String {
        lines.iter().map(|l| format!("{l}\n")).collect()
    }

    #[test]
    fn moved_text_is_found_again() {
        let old = long(7);
        // Lines 2 and 3 moved to the end of the file.
        let mut new: Vec<String> = [0, 1, 4, 5, 6, 2, 3].map(|i| old[i].clone()).to_vec();
        assert_eq!(placed(&file(&old), 2, 3, &file(&new)), Some((5, 6)));
        // Line 4 moved to the top.
        new = [4, 0, 1, 2, 3, 5, 6].map(|i| old[i].clone()).to_vec();
        assert_eq!(placed(&file(&old), 4, 4, &file(&new)), Some((0, 0)));
    }

    #[test]
    fn the_nearest_copy_wins_then_the_best_context() {
        // `fn render_header` at line 7 is rewritten, and a copy appears at line 9: nearer than
        // line 0's.
        let old = "fn render_header\n1\n2\n3\n4\n5\n6\nfn render_header\nx\n";
        let new = "fn render_header\n1\n2\n3\n4\n5\n6\nX\nx\nfn render_header\n";
        assert_eq!(placed(old, 7, 7, new), Some((9, 9)));
        // Copies at lines 1 and 5 are both two away from line 3; line 5 has the old context.
        let old = "p\nkey\nm\nkey\nm\nkey\nq\n";
        let new = "p\nkey\nm\nKEY\nm\nkey\nm\nkey\nq\n";
        assert_eq!(placed(old, 3, 3, new), Some((5, 5)));
    }

    #[test]
    fn rewritten_text_is_outdated() {
        assert_eq!(placed(F, 2, 3, "a\nb\nC\nd\ne\nf\ng\n"), None);
        // A line inserted inside the range is a small edit: the thread covers it too.
        assert_eq!(placed(F, 2, 3, "a\nb\nc\nnew\nd\ne\nf\ng\n"), Some((2, 4)));
        // Deleted, or the file emptied.
        assert_eq!(placed(F, 2, 3, "a\nb\ne\nf\ng\n"), None);
        assert_eq!(placed(F, 2, 3, ""), None);
    }

    #[test]
    fn line_endings_and_the_last_line() {
        // The last line with and without its newline is the same line.
        let old = "a\nb\nlast\n";
        assert_eq!(placed(old, 2, 2, "x\na\nb\nlast"), Some((3, 3)));
        // CRLF everywhere: every line differs as text, but the content is the same.
        assert_eq!(placed(old, 1, 1, "a\r\nb\r\nlast\r\n"), Some((1, 1)));
    }

    #[test]
    fn a_blank_line_finds_the_nearest_blank() {
        let old = "a\n\nb\n\nc\n";
        assert_eq!(placed(old, 3, 3, "a\n\nb\nB\n\nc\n"), Some((4, 4)));
    }

    /// A file of `n` long lines with `line` at `at`, and a copy of it at `copy` with other lines
    /// around it.
    fn with_copy(n: usize, at: usize, copy: usize, line: &str) -> (String, String) {
        let mut old = long(n);
        old[at] = line.into();
        let mut new = old.clone();
        new[at] = "let rewritten = something_else(42);".into();
        new[copy] = line.into();
        (file(&old), file(&new))
    }

    #[test]
    fn a_brace_doesnt_jump_to_another_brace() {
        let (old, new) = with_copy(600, 10, 510, "}");
        assert_eq!(placed(&old, 10, 10, &new), None);
        // Even near by, a copy of a short line needs a context line.
        let (old, new) = with_copy(600, 10, 12, "}");
        assert_eq!(placed(&old, 10, 10, &new), None);
    }

    #[test]
    fn a_copy_counts_only_within_200_lines() {
        let line = "let important = compute_the_total(items);";
        let (old, new) = with_copy(600, 10, 210, line);
        assert_eq!(placed(&old, 10, 10, &new), Some((210, 210)));
        let (old, new) = with_copy(600, 10, 211, line);
        assert_eq!(placed(&old, 10, 10, &new), None);
    }

    #[test]
    fn a_block_that_moved_far_with_its_context_keeps_its_thread() {
        // Eleven lines, a thread on the middle five with three lines of context either side,
        // move from the top to after 1,000 other lines.
        let block = long(11);
        let others: Vec<String> = (0..1000).map(|i| format!("let other{i} = {i};")).collect();
        let old = [block.clone(), others.clone()].concat();
        let mut new = [others, block].concat();
        assert_eq!(placed(&file(&old), 3, 7, &file(&new)), Some((1003, 1007)));
        // Without one of its context lines it's too far.
        new.remove(1010);
        assert_eq!(placed(&file(&old), 3, 7, &file(&new)), None);
    }

    #[test]
    fn a_short_line_needs_a_context_line() {
        // `ab = 1234` has seven characters that aren't spaces.
        let old = "fn first() {\n    let aa = 1;\n    ab = 1234\n    let bb = 2;\n}\n";
        let moved = "fn first() {\n    let aa = 1;\n    let bb = 2;\n}\n\
                     fn second() {\n    let cc = 3;\n    ab = 1234\n    let dd = 4;\n}\n";
        assert_eq!(placed(old, 2, 2, moved), None);
        let with_context = moved.replace("let cc = 3;", "let aa = 1;");
        assert_eq!(placed(old, 2, 2, &with_context), Some((6, 6)));
        // Eight characters is long enough to need none.
        let old = old.replace("ab = 1234", "abc = 1234");
        let moved = moved.replace("ab = 1234", "abc = 1234");
        assert_eq!(placed(&old, 2, 2, &moved), Some((6, 6)));
    }

    #[test]
    fn a_common_line_needs_a_context_line() {
        // `let total = 0;` changes at line 10, and copies elsewhere have other lines around them.
        let line = "let total = 0;";
        let copies = |n: usize| {
            let (old, new) = with_copy(100, 10, 30, line);
            let mut new: Vec<String> = new.lines().map(String::from).collect();
            for k in 1..n {
                new[30 + 10 * k] = line.into();
            }
            (old, file(&new))
        };
        let (old, new) = copies(3);
        assert_eq!(placed(&old, 10, 10, &new), Some((30, 30)));
        let (old, new) = copies(4);
        assert_eq!(placed(&old, 10, 10, &new), None);
    }

    #[test]
    fn a_small_edit_keeps_the_thread() {
        let old = "fn total(items: &[Item]) -> u32 {\n    items.iter().map(|i| i.prise).sum()\n}\n";
        let new = "fn total(items: &[Item]) -> u32 {\n    items.iter().map(|i| i.price).sum()\n}\n";
        assert_eq!(placed(old, 0, 2, new), Some((0, 2)));
        // A rewrite isn't the same code.
        let new = "fn total(list: Vec<Order>) -> f64 {\n    list.into_iter().fold(0.0, add)\n}\n";
        assert_eq!(placed(old, 0, 2, new), None);
    }

    #[test]
    fn similar_means_seven_tenths_of_the_words() {
        // Ten words, of which seven stay, is just similar enough; six isn't.
        let old = "one\nalpha beta gamma delta epsilon zeta eta theta iota kappa\ntwo\n";
        let seven = "one\nalpha beta gamma delta epsilon zeta eta X Y Z\ntwo\n";
        let six = "one\nalpha beta gamma delta epsilon zeta W X Y Z\ntwo\n";
        assert_eq!(placed(old, 1, 1, seven), Some((1, 1)));
        assert_eq!(placed(old, 1, 1, six), None);
    }
}
