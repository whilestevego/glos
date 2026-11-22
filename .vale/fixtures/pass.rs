//! A module doc in short sentences.

/// Returns the line count. The count includes the last line.
fn fixture() -> usize {
    let s = "a robust — string is code, and Vale skips it";
    // A short inline comment.
    s.len()
}
