//! `packages/utils/src/normalize-for-match.ts` (+ the shared NFD helper).

use icu_normalizer::DecomposingNormalizerBorrowed;

/// `value.normalize("NFD")`.
pub fn nfd(value: &str) -> String {
    DecomposingNormalizerBorrowed::new_nfd()
        .normalize(value)
        .into_owned()
}

/// Drops the Combining Diacritical Marks block (U+0300–U+036F).
pub fn strip_combining_marks(value: &str) -> String {
    value
        .chars()
        .filter(|c| !('\u{0300}'..='\u{036f}').contains(c))
        .collect()
}

/// `normalizeForMatch`: NFD, combining marks dropped, `đ`/`Đ` → `d`,
/// lower-cased.
pub fn normalize_for_match(value: &str) -> String {
    let stripped = strip_combining_marks(&nfd(value));
    let mapped: String = stripped
        .chars()
        .map(|c| if c == 'đ' || c == 'Đ' { 'd' } else { c })
        .collect();
    mapped.to_lowercase()
}
