//! The pure parts of `utils.ts`: placeholder extraction, interpolation with
//! position-dependent `{{gender}}` casing, the render time zones and the
//! date / custom-field formatting.

use std::collections::HashMap;
use std::sync::OnceLock;

use regex::Regex;

use super::VariableContext;
use super::system_field::{SystemFieldType, normalize_stored_timezone};
use crate::js;
use crate::utils::datetime::{
    DATE_FORMAT, DATE_TIME_FORMAT, DEFAULT_FILTER_TIMEZONE, format_custom_field_value_in_time_zone,
    format_with_fallback,
};
use crate::utils::tz::DateArg;
use crate::utils::variables::variable_placeholder_re;

/// `RAW_CUSTOM_FIELD_VARIABLE_PREFIX`.
pub const RAW_CUSTOM_FIELD_VARIABLE_PREFIX: &str = "raw:";

/// `SENTENCE_CASED_VARIABLES`: `{{gender}}` renders a salutation whose case
/// depends on where it sits.
const SENTENCE_CASED_VARIABLES: &[SystemFieldType] = &[SystemFieldType::Gender];

/// `SENTENCE_OPENING_RE`: the text before a placeholder that opens the
/// message, a line, or a sentence.
fn sentence_opening_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new("(?:^|[.!?…\\n\\r])[\\s\"'“‘(\\[]*$").expect("valid regex"))
}

/// `extractVariables`: the distinct trimmed placeholder names, in order of
/// first appearance.
pub fn extract_variables(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for caps in variable_placeholder_re().captures_iter(text) {
        let name = js::trim(&caps[1]).to_owned();
        if !out.contains(&name) {
            out.push(name);
        }
    }
    out
}

/// `interpolate`: each placeholder whose trimmed name is in `mapping` is
/// replaced; others stay literal. With `sentence_case`, `{{gender}}` keeps
/// its opening form at the start of a sentence and is lowercased inside one.
pub fn interpolate(text: &str, mapping: &HashMap<String, String>, sentence_case: bool) -> String {
    variable_placeholder_re()
        .replace_all(text, |caps: &regex::Captures<'_>| {
            let whole = caps.get(0).expect("match");
            let key = js::trim(&caps[1]);
            let Some(value) = mapping.get(key) else {
                return whole.as_str().to_owned();
            };
            let cased = sentence_case
                && SystemFieldType::parse(key)
                    .is_some_and(|k| SENTENCE_CASED_VARIABLES.contains(&k));
            if !cased || sentence_opening_re().is_match(&text[..whole.start()]) {
                value.clone()
            } else {
                js::to_lower(value)
            }
        })
        .into_owned()
}

/// `toRawCustomFieldName`: the name after `raw:`, untrimmed.
pub fn to_raw_custom_field_name(variable: &str) -> &str {
    variable
        .strip_prefix(RAW_CUSTOM_FIELD_VARIABLE_PREFIX)
        .unwrap_or(variable)
}

/// `getTimezone`: workspace first (workspace-scoped values such as
/// `current_time`), then the contact's stored zone.
pub fn get_timezone(context: &VariableContext) -> Option<String> {
    match context.workspace.as_ref().and_then(|w| w.timezone.clone()) {
        Some(timezone) => Some(timezone),
        None => normalize_stored_timezone(context.contact.timezone.as_deref()),
    }
}

/// `getContactTimezone`: the contact's zone, then the workspace's, then UTC.
pub fn get_contact_timezone(context: &VariableContext) -> String {
    normalize_stored_timezone(context.contact.timezone.as_deref())
        .or_else(|| context.workspace.as_ref().and_then(|w| w.timezone.clone()))
        .unwrap_or_else(|| DEFAULT_FILTER_TIMEZONE.to_owned())
}

/// `formatDate`: `yyyy-MM-dd` in `timezone`; `None` without a date.
pub fn format_date(date: Option<i64>, timezone: Option<&str>) -> Option<String> {
    date.map(|ms| format_with_fallback(DateArg::Ms(ms), timezone, DATE_FORMAT))
}

/// `formatDateTime`: `yyyy-MM-dd HH:mm:ss` in `timezone`.
pub fn format_date_time(date: Option<i64>, timezone: Option<&str>) -> Option<String> {
    date.map(|ms| format_with_fallback(DateArg::Ms(ms), timezone, DATE_TIME_FORMAT))
}

/// `renderCustomFieldValue`: a temporal value formatted in `timezone`
/// (UTC by default), a `date` as its literal calendar day, anything else
/// unchanged.
pub fn render_custom_field_value(
    kind: &str,
    value: Option<&str>,
    timezone: Option<&str>,
) -> String {
    format_custom_field_value_in_time_zone(kind, value, timezone.unwrap_or(DEFAULT_FILTER_TIMEZONE))
}

/// `capitalizeFirstLetter` (first UTF-16 unit, like `charAt(0)`).
pub fn capitalize_first_letter(value: Option<&str>) -> Option<String> {
    let value = value?;
    let mut chars = value.chars();
    match chars.next() {
        None => Some(String::new()),
        Some(first) => Some(format!(
            "{}{}",
            js::to_upper(&first.to_string()),
            chars.as_str()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    #[test]
    fn interpolate_trims_names_and_keeps_unknown_literal() {
        let m = map(&[("first_name", "Ada")]);
        assert_eq!(
            interpolate("Hi {{ first_name }} {{x}}", &m, true),
            "Hi Ada {{x}}"
        );
        assert_eq!(extract_variables("{{a}}{{ a }}{{b}}"), vec!["a", "b"]);
    }

    #[test]
    fn gender_is_cased_by_position_only_in_prose() {
        let m = map(&[("gender", "Anh")]);
        let text = "{{gender}} ơi. \"{{gender}}\" và {{gender}}";
        assert_eq!(interpolate(text, &m, true), "Anh ơi. \"Anh\" và anh");
        assert_eq!(interpolate(text, &m, false), "Anh ơi. \"Anh\" và Anh");
    }
}
