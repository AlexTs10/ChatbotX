//! Port of `packages/business/__tests__/pipeline-naming.test.ts` (pure
//! functions; `test.each` = one test over its cases).

use chatbotx_business::pipeline::naming::{SuffixStyle, allocate_unique_name, next_field_position};
use chatbotx_db::repo::partials::pipeline::reference_name_from_name;

fn valid_reference(value: &str) -> bool {
    let mut chars = value.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

#[test]
fn reference_name_from_name_becomes_the_expected_reference() {
    let cases = [
        ("Sales", "sales"),
        ("Customer Support", "customer_support"),
        ("Bán hàng", "ban_hang"),
        ("Đàm phán", "dam_phan"),
        ("Nguồn khách", "nguon_khach"),
        ("Hỗ trợ khách hàng", "ho_tro_khach_hang"),
        ("  --VIP!!  ", "vip"),
        ("2024 Plan", "pipeline_2024_plan"),
        ("", "pipeline"),
    ];
    for (name, expected) in cases {
        assert_eq!(
            reference_name_from_name(name, "pipeline"),
            expected,
            "{name:?}"
        );
    }
    // The naming module is the code under test of this file.
    assert_eq!(
        allocate_unique_name("sales", [], SuffixStyle::ReferenceName),
        "sales"
    );
}

#[test]
fn reference_name_from_name_never_exceeds_64_characters_and_stays_valid() {
    let reference = reference_name_from_name(&"a".repeat(200), "pipeline");
    assert_eq!(reference.len(), 64);
    assert!(valid_reference(&reference));
    assert_eq!(
        allocate_unique_name(&reference, [], SuffixStyle::ReferenceName),
        reference
    );
}

#[test]
fn allocate_unique_name_returns_the_candidate_when_it_is_free() {
    assert_eq!(
        allocate_unique_name("Sales", ["Support"], SuffixStyle::Name),
        "Sales"
    );
}

#[test]
fn allocate_unique_name_appends_the_first_free_number() {
    assert_eq!(
        allocate_unique_name("Sales", ["Sales", "Sales 2"], SuffixStyle::Name),
        "Sales 3"
    );
}

#[test]
fn allocate_unique_name_uses_an_underscore_suffix_for_reference_names() {
    assert_eq!(
        allocate_unique_name("sales", ["sales"], SuffixStyle::ReferenceName),
        "sales_2"
    );
}

#[test]
fn allocate_unique_name_truncates_the_base_so_the_suffixed_name_still_fits() {
    let long = "x".repeat(120);
    let next = allocate_unique_name(&long, [long.as_str()], SuffixStyle::Name);
    assert_eq!(next.len(), 120);
    assert!(next.ends_with(" 2"));
}

#[test]
fn next_field_position_goes_after_the_highest_position_not_the_count_when_a_field_was_deleted() {
    assert_eq!(next_field_position([1, 2]), 3);
    assert_eq!(next_field_position([]), 0);
}
