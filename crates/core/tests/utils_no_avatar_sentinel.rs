//! Port of `packages/business/src/media/__tests__/no-avatar-sentinel.test.ts`.

use chatbotx_core::utils::no_avatar_sentinel::{
    AVATAR_REFETCH_AFTER_MS, NO_AVATAR_SENTINEL_KEY, build_no_avatar_sentinel, has_real_avatar,
    is_no_avatar_sentinel_fresh, parse_no_avatar_sentinel,
};

const WINDOW: i64 = AVATAR_REFETCH_AFTER_MS as i64;

#[test]
fn builds_a_sentinel_with_the_failure_timestamp() {
    assert_eq!(
        build_no_avatar_sentinel(1234),
        format!("{NO_AVATAR_SENTINEL_KEY}?time=1234")
    );
}

#[test]
fn returns_null_for_a_real_avatar_key() {
    assert_eq!(
        parse_no_avatar_sentinel("public/avatars/contact-1.jpg"),
        None
    );
}

#[test]
fn treats_the_bare_placeholder_key_as_a_real_avatar_key() {
    assert_eq!(parse_no_avatar_sentinel(NO_AVATAR_SENTINEL_KEY), None);
    assert!(has_real_avatar(Some(NO_AVATAR_SENTINEL_KEY)));
}

#[test]
fn parses_a_valid_sentinel_timestamp() {
    assert_eq!(
        parse_no_avatar_sentinel(&build_no_avatar_sentinel(1234)),
        Some(1234.0)
    );
}

#[test]
fn treats_a_malformed_timestamp_as_stale() {
    for timestamp in ["invalid", "", "12ms"] {
        assert_eq!(
            parse_no_avatar_sentinel(&format!("{NO_AVATAR_SENTINEL_KEY}?time={timestamp}")),
            Some(0.0),
            "{timestamp:?}"
        );
    }
}

#[test]
fn is_fresh_immediately_before_the_refetch_boundary() {
    assert!(is_no_avatar_sentinel_fresh(1000.0, 1000 + WINDOW - 1));
}

#[test]
fn is_stale_at_the_refetch_boundary() {
    assert!(!is_no_avatar_sentinel_fresh(1000.0, 1000 + WINDOW));
}

#[test]
fn is_stale_when_the_failure_timestamp_is_in_the_future() {
    assert!(!is_no_avatar_sentinel_fresh(2000.0, 1000));
}

#[test]
fn identifies_only_non_sentinel_values_as_real_avatars() {
    assert!(!has_real_avatar(None));
    assert!(!has_real_avatar(Some("")));
    assert!(!has_real_avatar(Some(&build_no_avatar_sentinel(1234))));
    assert!(has_real_avatar(Some("public/avatars/contact-1.jpg")));
}
