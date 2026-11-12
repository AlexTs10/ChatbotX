//! `packages/utils/src/cache-keys.ts`.

/// `USER_QUOTA_LABEL`.
pub const USER_QUOTA_LABEL: &str = "user-quota";

/// `liveKeyFor`: `<label>-live:<id>`.
pub fn live_key_for(label: &str, id: &str) -> String {
    format!("{label}-live:{id}")
}

/// `cacheKeyFor`: `<label>:<id>`.
pub fn cache_key_for(label: &str, id: &str) -> String {
    format!("{label}:{id}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_keep_the_original_shapes() {
        assert_eq!(live_key_for(USER_QUOTA_LABEL, "u1"), "user-quota-live:u1");
        assert_eq!(cache_key_for(USER_QUOTA_LABEL, "u1"), "user-quota:u1");
    }
}
