//! Re-anchoring: placing a thread again after its file changed.
//!
//! The old position is mapped through the line diff between the file as it was when the thread
//! was anchored and the file now. If the diff doesn't touch the anchored lines they've only
//! moved. If it does, the anchored text is searched for in the new file, nearest the mapped
//! position first, with the context lines breaking ties. If it's nowhere, the thread is outdated.

use crate::diff;

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
    // ponytail: exact text only, and no limit on how far it moved (an open question in the plan).
    (0..=have.len().saturating_sub(want.len()))
        .filter(|&i| have.len() >= want.len() && have[i..i + want.len()] == want[..])
        .min_by_key(|&i| (i.abs_diff(mapped), usize::MAX - context(i)))
        .map(|i| (i, i + want.len() - 1))
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

    #[test]
    fn moved_text_is_found_again() {
        // `c d` moved to the end of the file.
        assert_eq!(placed(F, 2, 3, "a\nb\ne\nf\ng\nc\nd\n"), Some((5, 6)));
        // Moved to the top.
        assert_eq!(placed(F, 4, 4, "e\na\nb\nc\nd\nf\ng\n"), Some((0, 0)));
    }

    #[test]
    fn the_nearest_copy_wins_then_the_best_context() {
        // `fn f` at line 7 is rewritten, and a copy appears at line 9: nearer than line 0's.
        let old = "fn f\n1\n2\n3\n4\n5\n6\nfn f\nx\n";
        let new = "fn f\n1\n2\n3\n4\n5\n6\nX\nx\nfn f\n";
        assert_eq!(placed(old, 7, 7, new), Some((9, 9)));
        // Copies at lines 1 and 5 are both two away from line 3; line 5 has the old context.
        let old = "p\nkey\nm\nkey\nm\nkey\nq\n";
        let new = "p\nkey\nm\nKEY\nm\nkey\nm\nkey\nq\n";
        assert_eq!(placed(old, 3, 3, new), Some((5, 5)));
    }

    #[test]
    fn rewritten_text_is_outdated() {
        assert_eq!(placed(F, 2, 3, "a\nb\nC\nd\ne\nf\ng\n"), None);
        // A line inserted inside the range changes the anchored text.
        assert_eq!(placed(F, 2, 3, "a\nb\nc\nnew\nd\ne\nf\ng\n"), None);
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
}
