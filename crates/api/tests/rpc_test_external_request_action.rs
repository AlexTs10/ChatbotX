//! Port of `apps/builder/__tests__/test-external-request-action.test.ts`
//! (Q.P6-U4). Adapted: the mocked `externalRequestService.execute` becomes a
//! real request; an SSRF refusal is the "expected client-safe" error and a
//! refused connection the unexpected one.

mod rpc_common;

use rpc_common::{OUTSIDER_TOKEN, OWNER_TOKEN, WS, action, fixture};
use serde_json::{Value, json};

async fn run(token: &str, url: &str) -> (axum::http::StatusCode, Value) {
    let fx = fixture().await;
    let input = json!({ "body": { "bodyType": "json", "jsonBody": "{\"key\":\"value\"}" },
                        "headers": [], "method": "POST", "url": url });
    action(
        &fx,
        "actions",
        "testExternalRequestAction",
        token,
        json!({ "workspaceId": WS.to_string(), "args": [WS.to_string(), input] }),
    )
    .await
}

#[tokio::test]
async fn maps_an_external_request_failure_to_a_client_safe_error() {
    // A public IP literal (no DNS) on a closed port: the request fails.
    let (status, body) = run(OWNER_TOKEN, "http://93.184.216.34:1/request").await;
    assert!(status.is_client_error(), "{status} {body}");
    assert!(
        body.to_string()
            .contains("Unable to test the external request"),
        "{body}"
    );
}

#[tokio::test]
async fn preserves_expected_client_safe_external_request_errors() {
    let (status, body) = run(OWNER_TOKEN, "http://169.254.169.254/latest").await;
    assert!(status.is_client_error(), "{status} {body}");
    assert!(
        body.to_string()
            .contains("This URL is not allowed for external requests"),
        "{body}"
    );
}

#[tokio::test]
async fn an_outsider_is_refused() {
    let (status, _) = run(OUTSIDER_TOKEN, "http://169.254.169.254/latest").await;
    assert!(status.is_client_error());
}
