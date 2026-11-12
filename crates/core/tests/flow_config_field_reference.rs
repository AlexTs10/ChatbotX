//! Port of `packages/flow-config/__tests__/field-reference.test.ts`.

mod flow_config_support;
use flow_config_support::*;

use fc::FieldReference;

fn custom(key: &str) -> FieldReference {
    FieldReference::CustomField {
        key: key.to_owned(),
    }
}

fn bot(id: &str) -> FieldReference {
    FieldReference::BotField { id: id.to_owned() }
}

#[test]
fn parsefieldreference_parses_a_legacy_numeric_id_as_a_customfield_key() {
    assert_eq!(fc::parse_field_reference("123"), custom("123"));
}

#[test]
fn parsefieldreference_parses_a_legacy_plain_name_as_a_customfield_key() {
    assert_eq!(
        fc::parse_field_reference("cf-birthday"),
        custom("cf-birthday")
    );
}

#[test]
fn parsefieldreference_parses_a_well_formed_bot_field_token() {
    assert_eq!(fc::parse_field_reference("bot_field:42"), bot("42"));
}

#[test]
fn parsefieldreference_falls_back_to_customfield_for_a_malformed_token_with_no_id() {
    assert_eq!(
        fc::parse_field_reference("bot_field:"),
        custom("bot_field:")
    );
}

#[test]
fn parsefieldreference_falls_back_to_customfield_for_a_malformed_token_with_a_non_numeric_id() {
    assert_eq!(
        fc::parse_field_reference("bot_field:abc"),
        custom("bot_field:abc")
    );
}

#[test]
fn parsefieldreference_falls_back_to_customfield_for_a_partially_numeric_malformed_token() {
    assert_eq!(
        fc::parse_field_reference("bot_field:12x"),
        custom("bot_field:12x")
    );
}

#[test]
fn parsefieldreference_treats_an_empty_string_as_a_customfield_key() {
    assert_eq!(fc::parse_field_reference(""), custom(""));
}

#[test]
fn formatbotfieldreference_parsefieldreference_round_trip_format_then_parse_returns_the_original_id()
 {
    let formatted = fc::format_bot_field_reference("777");
    assert_eq!(formatted, "bot_field:777");
    assert_eq!(fc::parse_field_reference(&formatted), bot("777"));
}

#[test]
fn isbotfieldreference_returns_true_for_a_well_formed_token() {
    assert!(fc::is_bot_field_reference("bot_field:1"));
}

#[test]
fn isbotfieldreference_returns_false_for_a_malformed_token() {
    assert!(!fc::is_bot_field_reference("bot_field:"));
    assert!(!fc::is_bot_field_reference("bot_field:abc"));
}

#[test]
fn isbotfieldreference_returns_false_for_a_legacy_name_or_id() {
    assert!(!fc::is_bot_field_reference("123"));
    assert!(!fc::is_bot_field_reference("cf-birthday"));
    assert!(!fc::is_bot_field_reference(""));
}

fn reference_ok(value: &str) -> bool {
    fc::zod_field_reference().safe_parse(&json!(value)).success
}

#[test]
fn zodfieldreference_accepts_a_legacy_numeric_id() {
    assert!(reference_ok("123"));
}

#[test]
fn zodfieldreference_accepts_a_legacy_name_containing_spaces() {
    assert!(reference_ok("My Custom Field"));
}

#[test]
fn zodfieldreference_accepts_a_legacy_name_containing_a_colon() {
    assert!(reference_ok("my:field"));
}

#[test]
fn zodfieldreference_accepts_a_well_formed_bot_field_token() {
    assert!(reference_ok("bot_field:123"));
}

#[test]
fn zodfieldreference_rejects_an_empty_string() {
    assert!(!reference_ok(""));
}

#[test]
fn zodfieldreference_rejects_a_blank_whitespace_only_string() {
    assert!(!reference_ok("   "));
}

#[test]
fn zodfieldreference_rejects_a_malformed_reserved_prefix_token_with_no_id() {
    assert!(!reference_ok("bot_field:"));
}

#[test]
fn zodfieldreference_rejects_a_malformed_reserved_prefix_token_with_a_non_numeric_id() {
    assert!(!reference_ok("bot_field:abc"));
}

#[test]
fn reserved_field_name_guard_isreservedfieldname_flags_any_bot_field_prefixed_name() {
    assert!(fc::is_reserved_field_name("bot_field:123"));
    assert!(fc::is_reserved_field_name("bot_field:"));
    assert!(fc::is_reserved_field_name("bot_field:anything"));
}

#[test]
fn reserved_field_name_guard_isreservedfieldname_allows_ordinary_names() {
    assert!(!fc::is_reserved_field_name("Birthday"));
    assert!(!fc::is_reserved_field_name("bot_field"));
}

#[test]
fn reserved_field_name_guard_zodfieldname_accepts_an_ordinary_name() {
    assert!(fc::zod_field_name().safe_parse(&json!("Birthday")).success);
}

#[test]
fn reserved_field_name_guard_zodfieldname_rejects_a_name_colliding_with_the_reference_token_shape()
{
    assert!(
        !fc::zod_field_name()
            .safe_parse(&json!("bot_field:123"))
            .success
    );
}

#[test]
fn reserved_field_name_guard_zodfieldname_rejects_an_empty_name() {
    assert!(!fc::zod_field_name().safe_parse(&json!("")).success);
}

#[test]
fn reserved_field_name_guard_zodfieldname_rejects_a_name_over_255_characters() {
    assert!(
        !fc::zod_field_name()
            .safe_parse(&json!(rep("a", 256)))
            .success
    );
}
