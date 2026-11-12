//! `packages/utils/src/zod.ts`: the shared string validators, as checks.

use std::sync::OnceLock;

use regex::Regex;

/// `DIGITS_REGEX` (ASCII digits).
pub const DIGITS_REGEX: &str = r"^\d+$";

fn digits_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new("^[0-9]+$").expect("valid regex"))
}

const BIGINT_ID_MAX_DIGITS: usize = 18;

/// `zodBigintAsString`: digits only.
pub fn is_bigint_string(value: &str) -> bool {
    digits_re().is_match(value)
}

/// `zodBigintIdAsString`: digits only, at most 18.
pub fn is_bigint_id_string(value: &str) -> bool {
    is_bigint_string(value) && value.len() <= BIGINT_ID_MAX_DIGITS
}

/// `z.url().safeParse(value).success`: a WHATWG-parsable absolute URL.
pub fn is_url(value: &str) -> bool {
    url::Url::parse(value).is_ok()
}

/// `zodUrlWithVariables(message)`: a URL, or any value with a
/// `{{variable}}`; `Err(message)` otherwise (default `"Invalid URL"`).
pub fn validate_url_with_variables(value: &str, message: Option<&str>) -> Result<(), String> {
    if super::variables::contains_variable_placeholder(value) || is_url(value) {
        Ok(())
    } else {
        Err(message.unwrap_or("Invalid URL").to_owned())
    }
}
