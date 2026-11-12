//! `packages/utils/src/contact-filter-value-format.ts`: the value formats
//! a contact filter accepts.

use std::sync::OnceLock;

use regex::Regex;
use serde_json::Value;

use super::datetime::{date_part_of, is_real_calendar_date};
use crate::js;

/// `NUMERIC_VALUE_PATTERN`.
pub const NUMERIC_VALUE_PATTERN: &str = r"^-?\d+(\.\d+)?$";

/// `DATETIME_VALUE_PATTERN`.
pub const DATETIME_VALUE_PATTERN: &str = r"^\d{4}-(0[1-9]|1[0-2])-(0[1-9]|[12]\d|3[01])([T ]([01]\d|2[0-3]):[0-5]\d(:[0-5]\d(\.\d{1,6})?)?(Z|[+-]([01]\d|2[0-3]):?[0-5]\d)?)?$";

/// The compiled [`NUMERIC_VALUE_PATTERN`] (ASCII digits, as in JS).
pub fn numeric_value_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(&format!("(?-u){NUMERIC_VALUE_PATTERN}")).expect("valid regex"))
}

fn datetime_value_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(&format!("(?-u){DATETIME_VALUE_PATTERN}")).expect("valid regex"))
}

/// `isValidDateTimeFilterValue`: the pattern, a real calendar day and a
/// parseable date.
pub fn is_valid_date_time_filter_value(value: &str) -> bool {
    datetime_value_re().is_match(value)
        && is_real_calendar_date(date_part_of(value))
        && js::parse_date(value).is_some()
}

/// `valueContainsVariablePlaceholder`: a string (or an array of them)
/// containing `{{`. Objects are not searched.
pub fn value_contains_variable_placeholder(value: &Value) -> bool {
    match value {
        Value::String(text) => text.contains("{{"),
        Value::Array(items) => items.iter().any(value_contains_variable_placeholder),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn date_time_values_must_be_real_and_parseable() {
        assert!(is_valid_date_time_filter_value("2026-07-22"));
        assert!(is_valid_date_time_filter_value("2026-07-22T08:30:00Z"));
        assert!(is_valid_date_time_filter_value(
            "2026-07-22 08:30:00.123+07:00"
        ));
        assert!(!is_valid_date_time_filter_value("2026-02-30"));
        assert!(!is_valid_date_time_filter_value("2026-7-22"));
        assert!(numeric_value_re().is_match("-12.5"));
        assert!(!numeric_value_re().is_match("1e3"));
    }

    #[test]
    fn placeholders_are_found_in_strings_and_arrays_only() {
        assert!(value_contains_variable_placeholder(&json!("Hi {{name}}")));
        assert!(value_contains_variable_placeholder(&json!([
            "a",
            ["{{b}}"]
        ])));
        assert!(!value_contains_variable_placeholder(
            &json!({ "a": "{{b}}" })
        ));
        assert!(!value_contains_variable_placeholder(&Value::Null));
    }
}
