//! Auth of the channel-token and mobile-user routes, from the parity auth
//! matrices (`conversion/parity/tests/p1-05-channels-connections/00-auth.test.ts`
//! "API channel token auth", `p1-11-messages-messenger/00-auth.test.ts`
//! "me routes take only a mobile access token"). No vitest file covers
//! `channelApiTokenAuthMidddleware`; these are Rust-side checks of the same rules.

mod support;

use axum::http::HeaderMap;
use support::{MockDeps, TOKEN, auth_result, bearer, send};

#[tokio::test]
async fn channel_route_without_a_token_is_401_unauthorized() {
    let deps = MockDeps::default();
    for (method, path) in [
        ("GET", "/v1/channels/api/me"),
        ("POST", "/v1/channels/api/messages"),
    ] {
        let answer = send(&deps, method, path, &HeaderMap::new()).await;
        assert_eq!(answer.status, 401, "{path}");
        assert_eq!(answer.code(), "UNAUTHORIZED");
        assert_eq!(answer.body["defined"], true);
    }
}

#[tokio::test]
async fn a_workspace_token_is_not_a_channel_token() {
    let deps = MockDeps::with_lookup(Some(auth_result(None, "full")));
    let answer = send(&deps, "POST", "/v1/channels/api/typing", &bearer(TOKEN)).await;
    assert_eq!(answer.status, 401);
    assert_eq!(answer.code(), "UNAUTHORIZED");
}

#[tokio::test]
async fn me_routes_refuse_no_token_and_a_workspace_token() {
    let deps = MockDeps::with_lookup(Some(auth_result(None, "full")));
    for headers in [HeaderMap::new(), bearer(TOKEN)] {
        for (method, path) in [("GET", "/v1/me"), ("POST", "/v1/me/password")] {
            let answer = send(&deps, method, path, &headers).await;
            assert_eq!(answer.status, 401, "{path}");
            assert_eq!(answer.code(), "INVALID_CHATBOT_TOKEN");
            assert_eq!(answer.message(), "Invalid or missing access token");
            assert_eq!(answer.body["defined"], true);
        }
    }
}
