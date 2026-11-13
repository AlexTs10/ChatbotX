//! `lib/copy-name.ts`: the names of copies (`<name> - Copy`, `<name> _copy 2`, …).
//! Lengths count Unicode scalar values (JS counts UTF-16 units).

use std::collections::HashSet;

/// `MAX_COPY_NAME_ATTEMPTS`.
pub const MAX_COPY_NAME_ATTEMPTS: usize = 50;

/// `CopyNameStyle`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopyNameStyle {
    /// ` - Copy`.
    Dash,
    /// ` _copy`.
    Underscore,
}

/// `CopyNameOptions`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CopyNameOptions {
    pub style: CopyNameStyle,
    pub max_length: usize,
}

/// `copyName`: the name of copy number `attempt` (1-based).
pub fn copy_name(base: &str, attempt: usize, options: CopyNameOptions) -> String {
    let _ = (base, attempt, options);
    todo!("P3.6-D2")
}

/// `copyNamePrefix`: the prefix every copy name of `base` starts with.
pub fn copy_name_prefix(base: &str, options: CopyNameOptions) -> String {
    let _ = (base, options);
    todo!("P3.6-D2")
}

/// `freeCopyNames`: the copy names not in `taken`, lowest number first.
pub fn free_copy_names(
    base: &str,
    taken: &HashSet<String>,
    options: CopyNameOptions,
) -> Vec<String> {
    let _ = (base, taken, options);
    todo!("P3.6-D2")
}

/// `insertWithFreeCopyName`: tries the first two free names; a unique
/// violation (of `constraint`, when given) moves to the next one. `Ok(None)`
/// = every attempt was taken.
pub fn insert_with_free_copy_name<T, E>(
    base: &str,
    taken: &HashSet<String>,
    options: CopyNameOptions,
    constraint: Option<&str>,
    is_unique_violation: impl Fn(&E, Option<&str>) -> bool,
    insert: impl FnMut(&str) -> Result<T, E>,
) -> Result<Option<T>, E> {
    let _ = (
        base,
        taken,
        options,
        constraint,
        is_unique_violation,
        insert,
    );
    todo!("P3.6-D2")
}
