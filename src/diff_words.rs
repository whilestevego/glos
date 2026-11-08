//! Which words changed within changed lines, as GitHub and `delta` highlight them. This is what
//! makes a tint useful in a review: it shows the one number that changed in a long line.

use std::{collections::HashMap, ops::Range};

use imara_diff::{Algorithm, Diff, InternedInput};

use crate::diff::Hunk;

/// The byte ranges that changed in each line that was paired with one on the other side, by
/// 0-based line number. The ranges are within the line, which excludes its line ending.
#[derive(Default, Debug, PartialEq)]
pub struct Words {
    pub old: HashMap<usize, Vec<Range<usize>>>,
    pub new: HashMap<usize, Vec<Range<usize>>>,
}

/// Byte ranges within a line.
type Ranges = Vec<Range<usize>>;

/// Hunks with more lines than this, on both sides together, aren't paired. Pairing compares
/// every removed line with every added one, so a huge hunk would be slow to open.
const MAX_HUNK_LINES: usize = 500;

/// Longer lines aren't compared. They're usually minified or generated.
const MAX_LINE_BYTES: usize = 2000;

/// Lines less alike than this aren't paired when a hunk's two sides differ in length.
const MIN_SIMILARITY: f32 = 0.5;

/// The changed words in every hunk that both removes and adds lines.
pub fn changed(old: &str, new: &str, hunks: &[Hunk]) -> Words {
    let (old_lines, new_lines) = (lines(old), lines(new));
    let mut words = Words::default();
    for h in hunks {
        if h.before.is_empty()
            || h.after.is_empty()
            || h.before.len() + h.after.len() > MAX_HUNK_LINES
        {
            continue;
        }
        let side = |lines: &[&'_ str], r: &Range<u32>| -> Vec<usize> {
            (r.start as usize..(r.end as usize).min(lines.len())).collect()
        };
        let (olds, news) = (side(&old_lines, &h.before), side(&new_lines, &h.after));
        for (o, n) in pair(&olds, &news, &old_lines, &new_lines) {
            let (a, b) = (old_lines[o], new_lines[n]);
            if a.len() > MAX_LINE_BYTES || b.len() > MAX_LINE_BYTES {
                continue;
            }
            if let Some((ra, rb)) = diff_line(a, b) {
                words.old.insert(o, ra);
                words.new.insert(n, rb);
            }
        }
    }
    words
}

/// The text's lines without their endings.
fn lines(text: &str) -> Vec<&str> {
    text.split_inclusive('\n')
        .map(|l| l.strip_suffix('\n').unwrap_or(l))
        .map(|l| l.strip_suffix('\r').unwrap_or(l))
        .collect()
}

/// Pairs a hunk's removed lines with its added ones. With as many on each side, they pair in
/// order. Otherwise each removed line pairs with the most similar added line after the last
/// pair, if they're alike enough, and the rest stay unpaired.
fn pair(olds: &[usize], news: &[usize], old: &[&str], new: &[&str]) -> Vec<(usize, usize)> {
    if olds.len() == news.len() {
        return olds.iter().copied().zip(news.iter().copied()).collect();
    }
    let new_counts: Vec<_> = news.iter().map(|&n| counts(new[n])).collect();
    let mut pairs = vec![];
    let mut next = 0;
    for &o in olds {
        let a = counts(old[o]);
        let best = (next..news.len())
            .map(|j| (j, similarity(&a, &new_counts[j])))
            .filter(|(_, s)| *s >= MIN_SIMILARITY)
            .max_by(|x, y| x.1.total_cmp(&y.1));
        if let Some((j, _)) = best {
            pairs.push((o, news[j]));
            next = j + 1;
        }
    }
    pairs
}

/// How many times each word is in a line, whitespace left out.
fn counts(line: &str) -> HashMap<&str, usize> {
    let mut m: HashMap<&str, usize> = HashMap::new();
    for (t, _) in tokens(line).filter(|(t, _)| !t.trim().is_empty()) {
        *m.entry(t).or_default() += 1;
    }
    m
}

/// How alike two texts are, from 0 to 1, as [`similarity`] measures lines.
pub fn text_similarity(a: &str, b: &str) -> f32 {
    similarity(&counts(a), &counts(b))
}

/// How alike two lines are, from 0 to 1: the share of their words they have in common.
fn similarity(a: &HashMap<&str, usize>, b: &HashMap<&str, usize>) -> f32 {
    let total: usize = a.values().sum::<usize>() + b.values().sum::<usize>();
    if total == 0 {
        return 1.0;
    }
    let common: usize = a
        .iter()
        .map(|(t, n)| (*n).min(b.get(t).copied().unwrap_or(0)))
        .sum();
    2.0 * common as f32 / total as f32
}

/// The line split into runs of word characters, runs of whitespace, and single other
/// characters, each with its byte range. `foo(bar, 1)` is `foo`, `(`, `bar`, `,`, ` `, `1`, `)`.
fn tokens(line: &str) -> impl Iterator<Item = (&str, Range<usize>)> {
    #[derive(PartialEq)]
    enum Class {
        Word,
        Space,
        Other,
    }
    let class = |c: char| {
        if c.is_alphanumeric() || c == '_' {
            Class::Word
        } else if c.is_whitespace() {
            Class::Space
        } else {
            Class::Other
        }
    };
    let mut chars = line.char_indices().peekable();
    std::iter::from_fn(move || {
        let (start, c) = chars.next()?;
        let k = class(c);
        let mut end = start + c.len_utf8();
        if k != Class::Other {
            while let Some(&(i, d)) = chars.peek() {
                if class(d) != k {
                    break;
                }
                end = i + d.len_utf8();
                chars.next();
            }
        }
        Some((&line[start..end], start..end))
    })
}

/// The changed byte ranges on each side of a pair of lines, or `None` when there's nothing
/// worth showing: the lines are the same, or more than half of either was rewritten, which
/// would highlight most of it and help no one.
fn diff_line(a: &str, b: &str) -> Option<(Ranges, Ranges)> {
    let (ta, tb): (Vec<_>, Vec<_>) = (tokens(a).collect(), tokens(b).collect());
    let mut input = InternedInput::default();
    input.update_before(ta.iter().map(|(t, _)| *t));
    input.update_after(tb.iter().map(|(t, _)| *t));
    let mut diff = Diff::compute(Algorithm::Histogram, &input);
    diff.postprocess_no_heuristic(&input);
    let ranges = |line: &str, toks: &[(&str, Range<usize>)], changed: &dyn Fn(u32) -> bool| {
        let mut out: Vec<Range<usize>> = vec![];
        for (i, (_, r)) in toks.iter().enumerate() {
            if !changed(i as u32) {
                continue;
            }
            // Join ranges with only whitespace between them, so `foo bar` reads as one change.
            match out.last_mut() {
                Some(last) if line[last.end..r.start].trim().is_empty() => last.end = r.end,
                _ => out.push(r.clone()),
            }
        }
        // A highlighted space at either end shows nothing, so the ranges are trimmed, and a
        // change that was only whitespace is left to the line tint.
        out.into_iter()
            .filter_map(|r| {
                let text = &line[r.clone()];
                let start = r.start + (text.len() - text.trim_start().len());
                let end = r.end - (text.len() - text.trim_end().len());
                (start < end).then_some(start..end)
            })
            .collect::<Vec<_>>()
    };
    let ra = ranges(a, &ta, &|i| diff.is_removed(i));
    let rb = ranges(b, &tb, &|i| diff.is_added(i));
    if ra.is_empty() && rb.is_empty() {
        return None;
    }
    let rewritten = |line: &str, rs: &[Range<usize>]| {
        let changed: usize = rs.iter().map(|r| line[r.clone()].trim().len()).sum();
        changed * 2 > line.trim().len()
    };
    if rewritten(a, &ra) || rewritten(b, &rb) {
        return None;
    }
    Some((ra, rb))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::hunks;

    fn words(old: &str, new: &str) -> Words {
        changed(old, new, &hunks(old, new))
    }

    /// The changed text on each side of line `old_line` → `new_line`.
    fn shown<'a>(
        w: &Words,
        old: &'a str,
        new: &'a str,
        lines_: (usize, usize),
    ) -> (Vec<&'a str>, Vec<&'a str>) {
        let pick = |text: &'a str, line: usize, m: &HashMap<usize, Vec<Range<usize>>>| {
            let l = lines(text)[line];
            m.get(&line)
                .into_iter()
                .flatten()
                .map(|r| &l[r.clone()])
                .collect::<Vec<_>>()
        };
        (pick(old, lines_.0, &w.old), pick(new, lines_.1, &w.new))
    }

    #[test]
    fn only_the_changed_word_is_highlighted() {
        let (old, new) = ("let a = x + 1;\n", "let a = x + 2;\n");
        let w = words(old, new);
        assert_eq!(shown(&w, old, new, (0, 0)), (vec!["1"], vec!["2"]));
        // A word added in the middle shows only on the new side.
        let (old, new) = ("call(a, c)\n", "call(a, b, c)\n");
        let w = words(old, new);
        assert_eq!(shown(&w, old, new, (0, 0)), (vec![], vec!["b,"]));
    }

    #[test]
    fn neighbouring_changes_join_across_whitespace() {
        let (old, new) = (
            "one two three four five six\n",
            "one TWO THREE four five six\n",
        );
        let w = words(old, new);
        assert_eq!(
            shown(&w, old, new, (0, 0)),
            (vec!["two three"], vec!["TWO THREE"])
        );
    }

    #[test]
    fn tokens_split_words_spaces_and_punctuation() {
        let t: Vec<&str> = tokens("foo(bar, 1)").map(|(t, _)| t).collect();
        assert_eq!(t, ["foo", "(", "bar", ",", " ", "1", ")"]);
        // Unicode letters are word characters, and the ranges are bytes.
        let t: Vec<(&str, Range<usize>)> = tokens("née→été").collect();
        assert_eq!(t, [("née", 0..4), ("→", 4..7), ("été", 7..12)]);
        assert_eq!(tokens("").count(), 0);
    }

    #[test]
    fn equal_hunks_pair_in_order() {
        let old = "a = 1\nb = 2\nc = 3\n";
        let new = "a = 10\nb = 20\nc = 30\n";
        let w = words(old, new);
        for i in 0..3 {
            assert!(w.old.contains_key(&i) && w.new.contains_key(&i), "line {i}");
        }
    }

    #[test]
    fn unequal_hunks_pair_alike_lines_in_order() {
        // One line was edited and one was added after it.
        let old = "keep\nlet total = price * count;\nend\n";
        let new = "keep\nlet total = price * amount;\nlog(total);\nend\n";
        let w = words(old, new);
        assert_eq!(w.old.keys().collect::<Vec<_>>(), [&1]);
        assert_eq!(
            w.new.keys().collect::<Vec<_>>(),
            [&1],
            "the added line isn't paired"
        );
        assert_eq!(shown(&w, old, new, (1, 1)), (vec!["count"], vec!["amount"]));
        // Nothing alike: nothing paired.
        let w = words("x\nfoo bar baz\n", "x\nqux\nquux\n");
        assert!(w.old.is_empty() && w.new.is_empty());
    }

    #[test]
    fn rewritten_lines_get_no_words() {
        let w = words("let a = compute(x);\n", "return other_thing(y, z);\n");
        assert!(w.old.is_empty() && w.new.is_empty());
    }

    #[test]
    fn size_limits_are_respected() {
        let long_old = format!("{} 1\n", "x".repeat(MAX_LINE_BYTES));
        let long_new = format!("{} 2\n", "x".repeat(MAX_LINE_BYTES));
        assert!(
            words(&long_old, &long_new).new.is_empty(),
            "a line too long"
        );
        let n = MAX_HUNK_LINES / 2 + 1;
        let old: String = (0..n).map(|i| format!("v = {i}\n")).collect();
        let new: String = (0..n).map(|i| format!("v = {}\n", i + 1000)).collect();
        assert!(words(&old, &new).new.is_empty(), "a hunk too big");
        let n = MAX_HUNK_LINES / 2;
        let old: String = (0..n).map(|i| format!("v = {i}\n")).collect();
        let new: String = (0..n).map(|i| format!("v = {}\n", i + 1000)).collect();
        assert_eq!(words(&old, &new).new.len(), n, "a hunk just small enough");
    }

    #[test]
    fn pure_additions_and_removals_have_no_words() {
        assert_eq!(words("a\n", "a\nb\n"), Words::default());
        assert_eq!(words("a\nb\n", "a\n"), Words::default());
        // CRLF line endings don't count as a change in the words.
        assert_eq!(words("a = 1\r\n", "a = 1\n"), Words::default());
    }
}
