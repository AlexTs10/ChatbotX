//! Q.P5-fixD: `business_error` follows oRPC `validateORPCError` (a common
//! `invalidRequestData` without `issues` data is undefined) and shows
//! `validation.maxItemsReached` params as `{ max, feature }` were written.

use chatbotx_api::handlers::{CREATING, business_error};
use chatbotx_business::BusinessError;
use serde_json::json;

#[test]
fn invalid_request_data_without_issues_is_undefined() {
    let ssrf = BusinessError::exception(
        "[ssrf-guard] Webhook URL is not allowed: http://10.0.0.1/x",
        "invalidRequestData",
        422,
    );
    let out = business_error(ssrf, CREATING);
    assert_eq!(
        (out.code.as_str(), out.status, out.defined),
        ("invalidRequestData", 422, false)
    );

    let mut issues = BusinessError::exception("Input validation failed", "invalidRequestData", 422);
    if let BusinessError::Exception { data, .. } = &mut issues {
        *data = Some(json!({ "issues": [] }));
    }
    assert!(business_error(issues, CREATING).defined);
}

#[test]
fn max_items_reached_lists_max_first() {
    let mut error = BusinessError::validation("_", "validation.maxItemsReached");
    if let BusinessError::Exception { data, .. } = &mut error {
        *data = Some(json!({ "max": 100, "feature": "inboundWebhooks" }));
    }
    let out = business_error(error, CREATING);
    assert_eq!(
        out.message,
        "validation.maxItemsReached (max=100, feature=inboundWebhooks)"
    );
}
