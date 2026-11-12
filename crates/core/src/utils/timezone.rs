//! `packages/utils/src/timezone.ts`: legacy IANA aliases and strict zone
//! validation.

use super::tz::{INTL_SUPPORTED_TIME_ZONES, is_intl_time_zone};

/// `CANONICAL_BY_LEGACY_TIMEZONE`, in declaration order.
const CANONICAL_BY_LEGACY_TIMEZONE: &[(&str, &str)] = &[
    ("Asia/Saigon", "Asia/Ho_Chi_Minh"),
    ("Asia/Calcutta", "Asia/Kolkata"),
    ("Asia/Katmandu", "Asia/Kathmandu"),
    ("Asia/Rangoon", "Asia/Yangon"),
    ("Asia/Chongqing", "Asia/Shanghai"),
    ("Asia/Harbin", "Asia/Shanghai"),
    ("Asia/Istanbul", "Europe/Istanbul"),
    ("Europe/Kiev", "Europe/Kyiv"),
    ("Europe/Uzhgorod", "Europe/Kyiv"),
    ("Europe/Zaporozhye", "Europe/Kyiv"),
    ("America/Buenos_Aires", "America/Argentina/Buenos_Aires"),
    ("America/Godthab", "America/Nuuk"),
    ("Pacific/Ponape", "Pacific/Pohnpei"),
    ("Pacific/Truk", "Pacific/Chuuk"),
    ("Atlantic/Faeroe", "Atlantic/Faroe"),
    ("Africa/Asmera", "Africa/Asmara"),
    ("Australia/Canberra", "Australia/Sydney"),
    ("US", "America/New_York"),
];

/// `timezoneCandidates`: the value, then its known rename (either way).
pub fn timezone_candidates(timezone: &str) -> Vec<String> {
    if let Some((_, canonical)) = CANONICAL_BY_LEGACY_TIMEZONE
        .iter()
        .find(|(l, _)| *l == timezone)
    {
        return vec![timezone.to_owned(), (*canonical).to_owned()];
    }
    match CANONICAL_BY_LEGACY_TIMEZONE
        .iter()
        .find(|(_, c)| *c == timezone)
    {
        Some((legacy, _)) => vec![timezone.to_owned(), (*legacy).to_owned()],
        None => vec![timezone.to_owned()],
    }
}

/// `canonicalizeTimezone`.
pub fn canonicalize_timezone(timezone: &str) -> String {
    CANONICAL_BY_LEGACY_TIMEZONE
        .iter()
        .find(|(l, _)| *l == timezone)
        .map_or_else(|| timezone.to_owned(), |(_, c)| (*c).to_owned())
}

/// `isValidTimezone`: a named zone the runtime can format with, listed
/// (in any of its spellings) by `Intl.supportedValuesOf` or `UTC`; offsets
/// and unusable aliases (`US`) are rejected.
pub fn is_valid_timezone(timezone: &str) -> bool {
    is_intl_time_zone(timezone)
        && timezone_candidates(timezone)
            .iter()
            .any(|name| name == "UTC" || INTL_SUPPORTED_TIME_ZONES.contains(&name.as_str()))
}
