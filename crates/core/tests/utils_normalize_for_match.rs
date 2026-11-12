//! Port of `packages/utils/__tests__/normalize-for-match.test.ts`.

use chatbotx_core::utils::normalize_for_match::normalize_for_match;

/// Port of: packages/utils/__tests__/normalize-for-match.test.ts:5 "normalizeForMatch > folds case and Vietnamese diacritics"
#[test]
fn normalizeformatch_folds_case_and_vietnamese_diacritics() {
    assert_eq!(normalize_for_match("Xin Chào ĐẸP"), "xin chao dep");
}

/// Port of: packages/utils/__tests__/normalize-for-match.test.ts:9 "normalizeForMatch > treats accented and plain spellings as equal"
#[test]
fn normalizeformatch_treats_accented_and_plain_spellings_as_equal() {
    assert_eq!(normalize_for_match("café"), normalize_for_match("CAFE"));
}

/// Port of: packages/utils/__tests__/normalize-for-match.test.ts:13 "normalizeForMatch > keeps ASCII symbols that are not combining marks"
#[test]
fn normalizeformatch_keeps_ascii_symbols_that_are_not_combining_marks() {
    assert_eq!(normalize_for_match("^^ `x`"), "^^ `x`");
}

/// Port of: packages/utils/__tests__/normalize-for-match.test.ts:17 "normalizeForMatch > returns an empty string unchanged"
#[test]
fn normalizeformatch_returns_an_empty_string_unchanged() {
    assert_eq!(normalize_for_match(""), "");
}
