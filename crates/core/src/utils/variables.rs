//! `packages/utils/src/variables.ts`: the `{{variable}}` placeholder grammar.

use std::sync::OnceLock;

use regex::Regex;

/// `VARIABLE_PLACEHOLDER_SOURCE`.
pub const VARIABLE_PLACEHOLDER_SOURCE: &str = r"\{\{(coupon:[^{}\n]+|raw:[^{}\n]+|[^{}\n]+)\}\}";

/// The compiled placeholder pattern.
pub fn variable_placeholder_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(VARIABLE_PLACEHOLDER_SOURCE).expect("valid regex"))
}

/// `containsVariablePlaceholder`.
pub fn contains_variable_placeholder(value: &str) -> bool {
    variable_placeholder_re().is_match(value)
}
