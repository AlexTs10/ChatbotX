//! Port of `packages/utils/__tests__/web-url.test.ts`.

use chatbotx_core::utils::web_url::join_app_url;

/// Port of: packages/utils/__tests__/web-url.test.ts:5 "joinAppUrl > https://app.test joins with exactly one slash"
#[test]
fn joinappurl_https_app_test_joins_with_exactly_one_slash() {
    assert_eq!(
        join_app_url("https://app.test", "/shop/1"),
        "https://app.test/shop/1"
    );
}

/// Port of: packages/utils/__tests__/web-url.test.ts:5 "joinAppUrl > https://app.test/ joins with exactly one slash"
#[test]
fn joinappurl_https_app_test_joins_with_exactly_one_slash_2() {
    assert_eq!(
        join_app_url("https://app.test/", "/shop/1"),
        "https://app.test/shop/1"
    );
}

/// Port of: packages/utils/__tests__/web-url.test.ts:5 "joinAppUrl > https://app.test/// joins with exactly one slash"
#[test]
fn joinappurl_https_app_test_joins_with_exactly_one_slash_3() {
    assert_eq!(
        join_app_url("https://app.test///", "/shop/1"),
        "https://app.test/shop/1"
    );
}
