//! `pipeline/naming.ts`.

/// `SuffixStyle`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuffixStyle {
    /// `Sales 2` (max `PIPELINE_LIMITS.name`).
    Name,
    /// `sales_2` (max `PIPELINE_REFERENCE_NAME_MAX`).
    ReferenceName,
}

/// `allocateUniqueName`: `candidate`, or `candidate 2`, `candidate 3`… the
/// first one not in `taken` (lengths in UTF-16 units, as JS).
pub fn allocate_unique_name<'a>(
    candidate: &str,
    taken: impl IntoIterator<Item = &'a str>,
    style: SuffixStyle,
) -> String {
    let _ = (candidate, taken.into_iter().count(), style);
    todo!("P3.6-D1p")
}

/// `nextFieldPosition`: after the highest position, not the count.
pub fn next_field_position(positions: impl IntoIterator<Item = i64>) -> i64 {
    let _ = positions.into_iter().count();
    todo!("P3.6-D1p")
}
