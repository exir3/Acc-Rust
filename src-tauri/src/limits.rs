pub(crate) const MAX_DESCRIPTION: usize = 1000;
pub(crate) const MAX_LINE_DESCRIPTION: usize = 500;
pub(crate) const MAX_LINES: usize = 1000;
/// Largest amount accepted on one line (1e15 Rials); it stays below 2^53
/// and leaves room for sums of `MAX_LINES` lines inside an `i64`.
pub const MAX_AMOUNT: i64 = 999_999_999_999_999;
