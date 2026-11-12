//! `packages/utils/src/no-avatar-sentinel.ts`: the stored "no avatar"
//! marker, with the time the fetch failed.

use crate::js;

/// `NO_AVATAR_SENTINEL_KEY`.
pub const NO_AVATAR_SENTINEL_KEY: &str = "public/img/no_avatar.jpg";
/// `AVATAR_REFETCH_AFTER_MS`: a failed fetch is retried after 10 minutes.
pub const AVATAR_REFETCH_AFTER_MS: f64 = 10.0 * 60.0 * 1000.0;
/// `DEFAULT_AVATAR_PLACEHOLDER_PATH`.
pub const DEFAULT_AVATAR_PLACEHOLDER_PATH: &str = "/media/default-avatar.svg";

/// `buildNoAvatarSentinel(failedAtMs)`.
pub fn build_no_avatar_sentinel(failed_at_ms: i64) -> String {
    format!("{NO_AVATAR_SENTINEL_KEY}?time={failed_at_ms}")
}

/// `parseNoAvatarSentinel`: the failure time of a sentinel (`0` when it
/// is not a number), `None` for any other avatar.
pub fn parse_no_avatar_sentinel(avatar: &str) -> Option<f64> {
    let time = avatar
        .strip_prefix(NO_AVATAR_SENTINEL_KEY)?
        .strip_prefix("?time=")?;
    let failed_at_ms = js::to_number(time);
    Some(if failed_at_ms.is_finite() {
        failed_at_ms
    } else {
        0.0
    })
}

/// `hasRealAvatar`: a non-empty avatar that is not the sentinel.
pub fn has_real_avatar(avatar: Option<&str>) -> bool {
    avatar.is_some_and(|a| !a.is_empty() && parse_no_avatar_sentinel(a).is_none())
}

/// `isNoAvatarSentinelFresh`: failed within the last 10 minutes.
pub fn is_no_avatar_sentinel_fresh(failed_at_ms: f64, now_ms: i64) -> bool {
    let now = now_ms as f64;
    failed_at_ms <= now && now - failed_at_ms < AVATAR_REFETCH_AFTER_MS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sentinel_round_trips_and_ages() {
        let sentinel = build_no_avatar_sentinel(1_000);
        assert_eq!(sentinel, "public/img/no_avatar.jpg?time=1000");
        assert_eq!(parse_no_avatar_sentinel(&sentinel), Some(1_000.0));
        assert_eq!(
            parse_no_avatar_sentinel("public/img/no_avatar.jpg?time=x"),
            Some(0.0)
        );
        assert_eq!(
            parse_no_avatar_sentinel("public/img/no_avatar.jpg?time="),
            Some(0.0)
        );
        assert_eq!(parse_no_avatar_sentinel("avatars/a.png"), None);
        assert!(has_real_avatar(Some("avatars/a.png")));
        assert!(!has_real_avatar(Some(&sentinel)));
        assert!(!has_real_avatar(Some("")));
        assert!(!has_real_avatar(None));
        assert!(is_no_avatar_sentinel_fresh(1_000.0, 1_000 + 599_999));
        assert!(!is_no_avatar_sentinel_fresh(1_000.0, 1_000 + 600_000));
        assert!(!is_no_avatar_sentinel_fresh(2_000.0, 1_000));
    }
}
