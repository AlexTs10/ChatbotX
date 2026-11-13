//! Port of `@chatbotx.io/utils/contact-filter-value-format` (the parts the filter reads;
//! `chatbotx-db` cannot depend on `chatbotx-core`, which depends on it).

/// `isValidDateTimeFilterValue`: `YYYY-MM-DD[( |T)HH:MM[:SS[.ffffff]][Z|±HH[:]MM]]`, a
/// real calendar day.
pub fn is_valid_date_time_filter_value(value: &str) -> bool {
    let _ = value;
    todo!("P3.2b")
}
