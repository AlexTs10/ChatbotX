//! `pipeline/naming.ts`.

use std::collections::HashSet;

use chatbotx_db::repo::partials::pipeline::{PIPELINE_LIMITS, PIPELINE_REFERENCE_NAME_MAX};

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
    let used: HashSet<&str> = taken.into_iter().collect();
    if !used.contains(candidate) {
        return candidate.to_string();
    }
    let (separator, max) = match style {
        SuffixStyle::Name => (' ', PIPELINE_LIMITS.name),
        SuffixStyle::ReferenceName => ('_', PIPELINE_REFERENCE_NAME_MAX),
    };
    let units: Vec<u16> = candidate.encode_utf16().collect();
    for attempt in 2_u64.. {
        let suffix = format!("{separator}{attempt}");
        let keep = max.saturating_sub(suffix.len()).min(units.len());
        let next = format!("{}{suffix}", String::from_utf16_lossy(&units[..keep]));
        if !used.contains(next.as_str()) {
            return next;
        }
    }
    unreachable!("an unused suffix always exists")
}

/// `nextFieldPosition`: after the highest position, not the count.
pub fn next_field_position(positions: impl IntoIterator<Item = i64>) -> i64 {
    positions
        .into_iter()
        .fold(0, |next, position| next.max(position + 1))
}
