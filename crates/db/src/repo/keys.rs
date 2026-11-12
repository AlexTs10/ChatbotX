//! Port of `packages/database/src/keys.ts` (the env keys that still mean something with
//! one SQLite file: `DATABASE_URL`, pool size and shard settings do not).

/// The database env settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Keys {
    /// `CONTACT_ACTIVITY_FILTERS_ENABLED` (default `true`): kill switch for the
    /// activity-backed contact filters (off: they match nobody).
    pub contact_activity_filters_enabled: bool,
}

/// zod `stringbool()`: `true/1/yes/on/y/enabled` and `false/0/no/off/n/disabled`
/// (case-insensitive); other text is invalid (`None`).
pub fn parse_string_bool(raw: &str) -> Option<bool> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" | "on" | "y" | "enabled" => Some(true),
        "false" | "0" | "no" | "off" | "n" | "disabled" => Some(false),
        _ => None,
    }
}

/// `keys()`: reads the process env.
pub fn keys() -> Keys {
    Keys {
        contact_activity_filters_enabled: std::env::var("CONTACT_ACTIVITY_FILTERS_ENABLED")
            .ok()
            .and_then(|v| parse_string_bool(&v))
            .unwrap_or(true),
    }
}
