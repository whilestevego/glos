//! Integer conversions that saturate at the bounds of the target type in place of wrapping.

/// `n` as a count of terminal cells, or `u16::MAX` past it.
pub fn cells(n: usize) -> u16 {
    u16::try_from(n).unwrap_or(u16::MAX)
}

/// `n` as a `u32`, or `u32::MAX` past it.
pub fn to_u32(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// `n` as an `isize`, or `isize::MAX` past it.
pub fn to_isize(n: usize) -> isize {
    isize::try_from(n).unwrap_or(isize::MAX)
}

/// `n` as a `usize`, with a negative `n` as 0.
pub fn to_usize(n: i64) -> usize {
    usize::try_from(n.max(0)).unwrap_or(usize::MAX)
}

/// The 1-based line number of the 0-based line `index`.
pub fn line_number(index: usize) -> u32 {
    to_u32(index).saturating_add(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversions_saturate() {
        assert_eq!(cells(7), 7);
        assert_eq!(cells(70_000), u16::MAX);
        assert_eq!(to_u32(usize::MAX), u32::MAX);
        assert_eq!(to_isize(usize::MAX), isize::MAX);
        assert_eq!(to_usize(-3), 0);
        assert_eq!(to_usize(42), 42);
        assert_eq!(line_number(0), 1);
        assert_eq!(line_number(usize::MAX), u32::MAX);
    }
}
