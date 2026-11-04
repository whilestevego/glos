//! Line diffs with imara-diff, as Helix's diff gutter computes them.

use imara_diff::{Algorithm, Diff, InternedInput};

/// Lines `before` in the old text became lines `after` in the new one (0-based, end exclusive).
pub use imara_diff::Hunk;

pub fn hunks(old: &str, new: &str) -> Vec<Hunk> {
    let input = InternedInput::new(old, new);
    let mut diff = Diff::compute(Algorithm::Histogram, &input);
    diff.postprocess_lines(&input);
    diff.hunks().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_of_hunk() {
        let h = hunks("a\nb\nc\nd\n", "a\nB\nc\nnew\nd\n");
        assert_eq!(
            h,
            [
                Hunk {
                    before: 1..2,
                    after: 1..2
                },
                Hunk {
                    before: 3..3,
                    after: 3..4
                }
            ]
        );
        let removal = hunks("a\nb\nc\n", "a\nc\n");
        assert_eq!(
            removal,
            [Hunk {
                before: 1..2,
                after: 1..1
            }]
        );
    }
}
