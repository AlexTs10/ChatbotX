//! Port of `packages/utils/__tests__/custom-field.test.ts`.

use std::collections::HashSet;

use chatbotx_core::utils::custom_field::{
    MAX_CUSTOM_FIELD_NAME_LENGTH, allocate_renamed_custom_field_name, canonical_number_literal,
    coerce_boolean_literal, normalize_custom_field_name,
};

fn set(names: &[&str]) -> HashSet<String> {
    names.iter().map(|n| (*n).to_owned()).collect()
}

/// Port of: packages/utils/__tests__/custom-field.test.ts:17 "coerceBooleanLiteral > coerces true -> true"
#[test]
fn coercebooleanliteral_coerces_true_true() {
    assert_eq!(coerce_boolean_literal("true"), "true");
}

/// Port of: packages/utils/__tests__/custom-field.test.ts:17 "coerceBooleanLiteral > coerces TRUE -> true"
#[test]
fn coercebooleanliteral_coerces_true_true_2() {
    assert_eq!(coerce_boolean_literal("TRUE"), "true");
}

/// Port of: packages/utils/__tests__/custom-field.test.ts:17 "coerceBooleanLiteral > coerces 1 -> true"
#[test]
fn coercebooleanliteral_coerces_1_true() {
    assert_eq!(coerce_boolean_literal("1"), "true");
}

/// Port of: packages/utils/__tests__/custom-field.test.ts:17 "coerceBooleanLiteral > coerces yes -> true"
#[test]
fn coercebooleanliteral_coerces_yes_true() {
    assert_eq!(coerce_boolean_literal("yes"), "true");
}

/// Port of: packages/utils/__tests__/custom-field.test.ts:17 "coerceBooleanLiteral > coerces false -> false"
#[test]
fn coercebooleanliteral_coerces_false_false() {
    assert_eq!(coerce_boolean_literal("false"), "false");
}

/// Port of: packages/utils/__tests__/custom-field.test.ts:17 "coerceBooleanLiteral > coerces 0 -> false"
#[test]
fn coercebooleanliteral_coerces_0_false() {
    assert_eq!(coerce_boolean_literal("0"), "false");
}

/// Port of: packages/utils/__tests__/custom-field.test.ts:17 "coerceBooleanLiteral > coerces no -> false"
#[test]
fn coercebooleanliteral_coerces_no_false() {
    assert_eq!(coerce_boolean_literal("no"), "false");
}

/// Port of: packages/utils/__tests__/custom-field.test.ts:17 "coerceBooleanLiteral > coerces maybe -> true"
#[test]
fn coercebooleanliteral_coerces_maybe_true() {
    assert_eq!(coerce_boolean_literal("maybe"), "true");
}

/// Port of: packages/utils/__tests__/custom-field.test.ts:17 "coerceBooleanLiteral > coerces 12313 -> true"
#[test]
fn coercebooleanliteral_coerces_12313_true() {
    assert_eq!(coerce_boolean_literal("12313"), "true");
}

/// Port of: packages/utils/__tests__/custom-field.test.ts:32 "coerceBooleanLiteral > returns 'false' for blank input "" (defensive default — real callers guard blanks first)"
#[test]
fn coercebooleanliteral_returns_false_for_blank_input_defensive_default_real_callers_guard_blanks_first()
 {
    assert_eq!(coerce_boolean_literal(""), "false");
}

/// Port of: packages/utils/__tests__/custom-field.test.ts:32 "coerceBooleanLiteral > returns 'false' for blank input "   " (defensive default — real callers guard blanks first)"
#[test]
fn coercebooleanliteral_returns_false_for_blank_input_defensive_default_real_callers_guard_blanks_first_2()
 {
    assert_eq!(coerce_boolean_literal("   "), "false");
}

/// Port of: packages/utils/__tests__/custom-field.test.ts:46 "canonicalNumberLiteral > normalizes 7 -> 7"
#[test]
fn canonicalnumberliteral_normalizes_7_7() {
    assert_eq!(canonical_number_literal("7").as_deref(), Some("7"));
}

/// Port of: packages/utils/__tests__/custom-field.test.ts:46 "canonicalNumberLiteral > normalizes 007 -> 7"
#[test]
fn canonicalnumberliteral_normalizes_007_7() {
    assert_eq!(canonical_number_literal("007").as_deref(), Some("7"));
}

/// Port of: packages/utils/__tests__/custom-field.test.ts:46 "canonicalNumberLiteral > normalizes -2.5 -> -2.5"
#[test]
fn canonicalnumberliteral_normalizes_2_5_2_5() {
    assert_eq!(canonical_number_literal("-2.5").as_deref(), Some("-2.5"));
}

/// Port of: packages/utils/__tests__/custom-field.test.ts:46 "canonicalNumberLiteral > normalizes 0 -> 0"
#[test]
fn canonicalnumberliteral_normalizes_0_0() {
    assert_eq!(canonical_number_literal("0").as_deref(), Some("0"));
}

/// Port of: packages/utils/__tests__/custom-field.test.ts:46 "canonicalNumberLiteral > normalizes +1 -> 1"
#[test]
fn canonicalnumberliteral_normalizes_1_1() {
    assert_eq!(canonical_number_literal("+1").as_deref(), Some("1"));
}

/// Port of: packages/utils/__tests__/custom-field.test.ts:46 "canonicalNumberLiteral > normalizes 1. -> 1"
#[test]
fn canonicalnumberliteral_normalizes_1_1_2() {
    assert_eq!(canonical_number_literal("1.").as_deref(), Some("1"));
}

/// Port of: packages/utils/__tests__/custom-field.test.ts:46 "canonicalNumberLiteral > normalizes  1.5  -> 1.5"
#[test]
fn canonicalnumberliteral_normalizes_1_5_1_5() {
    assert_eq!(canonical_number_literal(" 1.5 ").as_deref(), Some("1.5"));
}

/// Port of: packages/utils/__tests__/custom-field.test.ts:46 "canonicalNumberLiteral > normalizes 1e3 -> 1000"
#[test]
fn canonicalnumberliteral_normalizes_1e3_1000() {
    assert_eq!(canonical_number_literal("1e3").as_deref(), Some("1000"));
}

/// Port of: packages/utils/__tests__/custom-field.test.ts:46 "canonicalNumberLiteral > normalizes 0x10 -> 16"
#[test]
fn canonicalnumberliteral_normalizes_0x10_16() {
    assert_eq!(canonical_number_literal("0x10").as_deref(), Some("16"));
}

/// Port of: packages/utils/__tests__/custom-field.test.ts:61 "canonicalNumberLiteral > returns null for blank input """
#[test]
fn canonicalnumberliteral_returns_null_for_blank_input() {
    assert_eq!(canonical_number_literal(""), None);
}

/// Port of: packages/utils/__tests__/custom-field.test.ts:61 "canonicalNumberLiteral > returns null for blank input "   ""
#[test]
fn canonicalnumberliteral_returns_null_for_blank_input_2() {
    assert_eq!(canonical_number_literal("   "), None);
}

/// Port of: packages/utils/__tests__/custom-field.test.ts:65 "canonicalNumberLiteral > returns null for unparseable/non-finite input 1aaa1"
#[test]
fn canonicalnumberliteral_returns_null_for_unparseable_non_finite_input_1aaa1() {
    assert_eq!(canonical_number_literal("1aaa1"), None);
}

/// Port of: packages/utils/__tests__/custom-field.test.ts:65 "canonicalNumberLiteral > returns null for unparseable/non-finite input abc"
#[test]
fn canonicalnumberliteral_returns_null_for_unparseable_non_finite_input_abc() {
    assert_eq!(canonical_number_literal("abc"), None);
}

/// Port of: packages/utils/__tests__/custom-field.test.ts:65 "canonicalNumberLiteral > returns null for unparseable/non-finite input Infinity"
#[test]
fn canonicalnumberliteral_returns_null_for_unparseable_non_finite_input_infinity() {
    assert_eq!(canonical_number_literal("Infinity"), None);
}

/// Port of: packages/utils/__tests__/custom-field.test.ts:65 "canonicalNumberLiteral > returns null for unparseable/non-finite input -Infinity"
#[test]
fn canonicalnumberliteral_returns_null_for_unparseable_non_finite_input_infinity_2() {
    assert_eq!(canonical_number_literal("-Infinity"), None);
}

/// Port of: packages/utils/__tests__/custom-field.test.ts:65 "canonicalNumberLiteral > returns null for unparseable/non-finite input NaN"
#[test]
fn canonicalnumberliteral_returns_null_for_unparseable_non_finite_input_nan() {
    assert_eq!(canonical_number_literal("NaN"), None);
}

/// Port of: packages/utils/__tests__/custom-field.test.ts:77 "allocateRenamedCustomFieldName > takes the smallest suffix whose normalized name is free"
#[test]
fn allocaterenamedcustomfieldname_takes_the_smallest_suffix_whose_normalized_name_is_free() {
    let taken = set(&["size", "size_1", "size_2"]);
    assert_eq!(
        allocate_renamed_custom_field_name("Size", &taken, MAX_CUSTOM_FIELD_NAME_LENGTH),
        "Size_3"
    );
}

/// Port of: packages/utils/__tests__/custom-field.test.ts:83 "allocateRenamedCustomFieldName > compares case- and whitespace-insensitively across every type"
#[test]
fn allocaterenamedcustomfieldname_compares_case_and_whitespace_insensitively_across_every_type() {
    let taken: HashSet<String> = [normalize_custom_field_name("  FOO_1 ")]
        .into_iter()
        .collect();
    assert_eq!(
        allocate_renamed_custom_field_name("foo", &taken, MAX_CUSTOM_FIELD_NAME_LENGTH),
        "foo_2"
    );
}

/// Port of: packages/utils/__tests__/custom-field.test.ts:89 "allocateRenamedCustomFieldName > shortens the base so the suffixed name stays within the limit"
#[test]
fn allocaterenamedcustomfieldname_shortens_the_base_so_the_suffixed_name_stays_within_the_limit() {
    let renamed = allocate_renamed_custom_field_name("abcdefghij", &HashSet::new(), 8);
    assert_eq!(renamed, "abcdef_1");
    assert_eq!(renamed.len(), 8);
}

/// Port of: packages/utils/__tests__/custom-field.test.ts:96 "allocateRenamedCustomFieldName > does not change the stored spelling of the base"
#[test]
fn allocaterenamedcustomfieldname_does_not_change_the_stored_spelling_of_the_base() {
    assert_eq!(
        allocate_renamed_custom_field_name(
            "Date of Birth",
            &HashSet::new(),
            MAX_CUSTOM_FIELD_NAME_LENGTH
        ),
        "Date of Birth_1"
    );
}
