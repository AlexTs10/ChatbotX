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

fn suffix_for(style: CopyNameStyle, attempt: usize) -> String {
    let suffix = match style {
        CopyNameStyle::Dash => " - Copy",
        CopyNameStyle::Underscore => " _copy",
    };
    if attempt == 1 {
        suffix.to_string()
    } else {
        format!("{suffix} {attempt}")
    }
}

/// The first `n` characters of `text`.
fn head(text: &str, n: usize) -> &str {
    match text.char_indices().nth(n) {
        Some((at, _)) => &text[..at],
        None => text,
    }
}

/// `copyName`: the name of copy number `attempt` (1-based).
pub fn copy_name(base: &str, attempt: usize, options: CopyNameOptions) -> String {
    let suffix = suffix_for(options.style, attempt);
    let keep = options.max_length.saturating_sub(suffix.chars().count());
    format!("{}{suffix}", head(base, keep))
}

/// `copyNamePrefix`: the prefix every copy name of `base` starts with.
pub fn copy_name_prefix(base: &str, options: CopyNameOptions) -> String {
    let longest = suffix_for(options.style, MAX_COPY_NAME_ATTEMPTS)
        .chars()
        .count();
    let room = options.max_length.saturating_sub(longest);
    if base.chars().count() <= room {
        return format!("{base}{}", suffix_for(options.style, 1));
    }
    head(base, room).to_string()
}

/// `freeCopyNames`: the copy names not in `taken`, lowest number first.
pub fn free_copy_names(
    base: &str,
    taken: &HashSet<String>,
    options: CopyNameOptions,
) -> Vec<String> {
    (1..=MAX_COPY_NAME_ATTEMPTS)
        .map(|attempt| copy_name(base, attempt, options))
        .filter(|name| !taken.contains(name))
        .collect()
}

const COPY_NAME_INSERT_ATTEMPTS: usize = 2;

/// `insertWithFreeCopyName`: tries the first two free names; a unique
/// violation (of `constraint`, when given) moves to the next one. `Ok(None)`
/// = every attempt was taken.
pub fn insert_with_free_copy_name<T, E>(
    base: &str,
    taken: &HashSet<String>,
    options: CopyNameOptions,
    constraint: Option<&str>,
    is_unique_violation: impl Fn(&E, Option<&str>) -> bool,
    mut insert: impl FnMut(&str) -> Result<T, E>,
) -> Result<Option<T>, E> {
    for name in free_copy_names(base, taken, options)
        .iter()
        .take(COPY_NAME_INSERT_ATTEMPTS)
    {
        match insert(name) {
            Ok(value) => return Ok(Some(value)),
            Err(error) if is_unique_violation(&error, constraint) => {}
            Err(error) => return Err(error),
        }
    }
    Ok(None)
}
