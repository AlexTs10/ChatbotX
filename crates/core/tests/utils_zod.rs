//! Port of `packages/utils/__tests__/zod.test.ts`. `schema.safeParse(v)`
//! becomes the check function's result.

use chatbotx_core::utils::variables::contains_variable_placeholder;
use chatbotx_core::utils::zod::{
    is_bigint_id_string, is_bigint_string, validate_url_with_variables,
};

/// Port of: packages/utils/__tests__/zod.test.ts:12 "zodBigintAsString > accepts "123""
#[test]
fn zodbigintasstring_accepts_123() {
    assert!(is_bigint_string("123"));
}

/// Port of: packages/utils/__tests__/zod.test.ts:12 "zodBigintAsString > accepts "0""
#[test]
fn zodbigintasstring_accepts_0() {
    assert!(is_bigint_string("0"));
}

/// Port of: packages/utils/__tests__/zod.test.ts:19 "zodBigintAsString > rejects "abc123""
#[test]
fn zodbigintasstring_rejects_abc123() {
    assert!(!is_bigint_string("abc123"));
}

/// Port of: packages/utils/__tests__/zod.test.ts:19 "zodBigintAsString > rejects "12a""
#[test]
fn zodbigintasstring_rejects_12a() {
    assert!(!is_bigint_string("12a"));
}

/// Port of: packages/utils/__tests__/zod.test.ts:19 "zodBigintAsString > rejects """
#[test]
fn zodbigintasstring_rejects() {
    assert!(!is_bigint_string(""));
}

/// Port of: packages/utils/__tests__/zod.test.ts:19 "zodBigintAsString > rejects " 12""
#[test]
fn zodbigintasstring_rejects_12() {
    assert!(!is_bigint_string(" 12"));
}

/// Port of: packages/utils/__tests__/zod.test.ts:27 "zodBigintIdAsString > accepts "1""
#[test]
fn zodbigintidasstring_accepts_1() {
    assert!(is_bigint_id_string("1"));
}

/// Port of: packages/utils/__tests__/zod.test.ts:27 "zodBigintIdAsString > accepts "0""
#[test]
fn zodbigintidasstring_accepts_0() {
    assert!(is_bigint_id_string("0"));
}

/// Port of: packages/utils/__tests__/zod.test.ts:27 "zodBigintIdAsString > accepts "999999999999999999""
#[test]
fn zodbigintidasstring_accepts_999999999999999999() {
    assert!(is_bigint_id_string("999999999999999999"));
}

/// Port of: packages/utils/__tests__/zod.test.ts:33 "zodBigintIdAsString > rejects "9999999999999999999""
#[test]
fn zodbigintidasstring_rejects_9999999999999999999() {
    assert!(!is_bigint_id_string("9999999999999999999"));
}

/// Port of: packages/utils/__tests__/zod.test.ts:33 "zodBigintIdAsString > rejects "abc""
#[test]
fn zodbigintidasstring_rejects_abc() {
    assert!(!is_bigint_id_string("abc"));
}

/// Port of: packages/utils/__tests__/zod.test.ts:33 "zodBigintIdAsString > rejects """
#[test]
fn zodbigintidasstring_rejects() {
    assert!(!is_bigint_id_string(""));
}

/// Port of: packages/utils/__tests__/zod.test.ts:33 "zodBigintIdAsString > rejects "12a""
#[test]
fn zodbigintidasstring_rejects_12a() {
    assert!(!is_bigint_id_string("12a"));
}

/// Port of: packages/utils/__tests__/zod.test.ts:42 "containsVariablePlaceholder > detects a placeholder in "{{booking_link}}""
#[test]
fn containsvariableplaceholder_detects_a_placeholder_in_booking_link() {
    assert!(contains_variable_placeholder("{{booking_link}}"));
}

/// Port of: packages/utils/__tests__/zod.test.ts:42 "containsVariablePlaceholder > detects a placeholder in "https://example.com/{{id}}""
#[test]
fn containsvariableplaceholder_detects_a_placeholder_in_https_example_com_id() {
    assert!(contains_variable_placeholder("https://example.com/{{id}}"));
}

/// Port of: packages/utils/__tests__/zod.test.ts:42 "containsVariablePlaceholder > detects a placeholder in "{{raw:field}}""
#[test]
fn containsvariableplaceholder_detects_a_placeholder_in_raw_field() {
    assert!(contains_variable_placeholder("{{raw:field}}"));
}

/// Port of: packages/utils/__tests__/zod.test.ts:42 "containsVariablePlaceholder > detects a placeholder in "{{coupon:SUMMER}}""
#[test]
fn containsvariableplaceholder_detects_a_placeholder_in_coupon_summer() {
    assert!(contains_variable_placeholder("{{coupon:SUMMER}}"));
}

/// Port of: packages/utils/__tests__/zod.test.ts:42 "containsVariablePlaceholder > detects a placeholder in "{{ domain }}""
#[test]
fn containsvariableplaceholder_detects_a_placeholder_in_domain() {
    assert!(contains_variable_placeholder("{{ domain }}"));
}

/// Port of: packages/utils/__tests__/zod.test.ts:54 "containsVariablePlaceholder > returns false for "https://example.com""
#[test]
fn containsvariableplaceholder_returns_false_for_https_example_com() {
    assert!(!contains_variable_placeholder("https://example.com"));
}

/// Port of: packages/utils/__tests__/zod.test.ts:54 "containsVariablePlaceholder > returns false for "adasdad""
#[test]
fn containsvariableplaceholder_returns_false_for_adasdad() {
    assert!(!contains_variable_placeholder("adasdad"));
}

/// Port of: packages/utils/__tests__/zod.test.ts:54 "containsVariablePlaceholder > returns false for "{{""
#[test]
fn containsvariableplaceholder_returns_false_for() {
    assert!(!contains_variable_placeholder("{{"));
}

/// Port of: packages/utils/__tests__/zod.test.ts:54 "containsVariablePlaceholder > returns false for "}}""
#[test]
fn containsvariableplaceholder_returns_false_for_2() {
    assert!(!contains_variable_placeholder("}}"));
}

/// Port of: packages/utils/__tests__/zod.test.ts:54 "containsVariablePlaceholder > returns false for "{{\n}}""
#[test]
fn containsvariableplaceholder_returns_false_for_n() {
    assert!(!contains_variable_placeholder("{{\n}}"));
}

/// Port of: packages/utils/__tests__/zod.test.ts:54 "containsVariablePlaceholder > returns false for """
#[test]
fn containsvariableplaceholder_returns_false_for_3() {
    assert!(!contains_variable_placeholder(""));
}

/// Port of: packages/utils/__tests__/zod.test.ts:69 "zodUrlWithVariables > accepts "https://example.com""
#[test]
fn zodurlwithvariables_accepts_https_example_com() {
    assert!(validate_url_with_variables("https://example.com", None).is_ok());
}

/// Port of: packages/utils/__tests__/zod.test.ts:69 "zodUrlWithVariables > accepts "https://example.com/path?x=1""
#[test]
fn zodurlwithvariables_accepts_https_example_com_path_x_1() {
    assert!(validate_url_with_variables("https://example.com/path?x=1", None).is_ok());
}

/// Port of: packages/utils/__tests__/zod.test.ts:69 "zodUrlWithVariables > accepts "{{booking_link}}""
#[test]
fn zodurlwithvariables_accepts_booking_link() {
    assert!(validate_url_with_variables("{{booking_link}}", None).is_ok());
}

/// Port of: packages/utils/__tests__/zod.test.ts:69 "zodUrlWithVariables > accepts "https://example.com/{{id}}""
#[test]
fn zodurlwithvariables_accepts_https_example_com_id() {
    assert!(validate_url_with_variables("https://example.com/{{id}}", None).is_ok());
}

/// Port of: packages/utils/__tests__/zod.test.ts:69 "zodUrlWithVariables > accepts "https://{{domain}}.com""
#[test]
fn zodurlwithvariables_accepts_https_domain_com() {
    assert!(validate_url_with_variables("https://{{domain}}.com", None).is_ok());
}

/// Port of: packages/utils/__tests__/zod.test.ts:69 "zodUrlWithVariables > accepts "{{ domain }}""
#[test]
fn zodurlwithvariables_accepts_domain() {
    assert!(validate_url_with_variables("{{ domain }}", None).is_ok());
}

/// Port of: packages/utils/__tests__/zod.test.ts:80 "zodUrlWithVariables > rejects "adasdad""
#[test]
fn zodurlwithvariables_rejects_adasdad() {
    assert!(validate_url_with_variables("adasdad", None).is_err());
}

/// Port of: packages/utils/__tests__/zod.test.ts:80 "zodUrlWithVariables > rejects "not a url""
#[test]
fn zodurlwithvariables_rejects_not_a_url() {
    assert!(validate_url_with_variables("not a url", None).is_err());
}

/// Port of: packages/utils/__tests__/zod.test.ts:80 "zodUrlWithVariables > rejects "{{""
#[test]
fn zodurlwithvariables_rejects() {
    assert!(validate_url_with_variables("{{", None).is_err());
}

/// Port of: packages/utils/__tests__/zod.test.ts:80 "zodUrlWithVariables > rejects "{{\n}}""
#[test]
fn zodurlwithvariables_rejects_n() {
    assert!(validate_url_with_variables("{{\n}}", None).is_err());
}

/// Port of: packages/utils/__tests__/zod.test.ts:80 "zodUrlWithVariables > rejects """
#[test]
fn zodurlwithvariables_rejects_2() {
    assert!(validate_url_with_variables("", None).is_err());
}

/// Port of: packages/utils/__tests__/zod.test.ts:91 "zodUrlWithVariables > reports the default Invalid URL message"
#[test]
fn zodurlwithvariables_reports_the_default_invalid_url_message() {
    assert_eq!(
        validate_url_with_variables("adasdad", None),
        Err("Invalid URL".to_owned())
    );
}

/// Port of: packages/utils/__tests__/zod.test.ts:99 "zodUrlWithVariables > honors a custom message"
#[test]
fn zodurlwithvariables_honors_a_custom_message() {
    assert_eq!(
        validate_url_with_variables("nope", Some("Bad link")),
        Err("Bad link".to_owned())
    );
}
