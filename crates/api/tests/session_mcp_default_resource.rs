//! Port of `packages/auth/__tests__/mcp-default-resource.test.ts`.

use std::collections::BTreeSet;

use chatbotx_api::session::{McpResourceRewrite, build_mcp_plugin_options, default_mcp_resource};
use serde_json::{Map, Value, json};

const RESOURCE: &str = "https://app.example.com/mcp";

fn map(value: Value) -> Map<String, Value> {
    value.as_object().expect("object").clone()
}

fn authorize(query: Value) -> Option<McpResourceRewrite> {
    default_mcp_resource("/oauth2/authorize", Some(&map(query)), None, RESOURCE)
}

fn token(body: Value) -> Option<McpResourceRewrite> {
    default_mcp_resource("/oauth2/token", None, Some(&map(body)), RESOURCE)
}

#[test]
fn caches_the_configured_oauth_resource() {
    assert_eq!(
        build_mcp_plugin_options(RESOURCE).cached_resources,
        BTreeSet::from([RESOURCE.to_string()])
    );
}

#[test]
fn injects_the_resource_into_an_authorize_query_that_omits_it() {
    assert_eq!(
        authorize(json!({ "client_id": "c" })),
        Some(McpResourceRewrite::Query(map(
            json!({ "client_id": "c", "resource": RESOURCE })
        )))
    );
}

#[test]
fn injects_the_resource_into_a_token_body_that_omits_it() {
    assert_eq!(
        token(json!({ "code": "x" })),
        Some(McpResourceRewrite::Body(map(
            json!({ "code": "x", "resource": RESOURCE })
        )))
    );
}

#[test]
fn normalizes_chatgpt_s_legacy_sse_endpoint_to_the_registered_resource() {
    assert_eq!(
        authorize(json!({ "client_id": "c", "resource": format!("{RESOURCE}/sse") })),
        Some(McpResourceRewrite::Query(map(
            json!({ "client_id": "c", "resource": RESOURCE })
        )))
    );
}

#[test]
fn normalizes_the_streamable_http_endpoint_to_the_registered_resource() {
    assert_eq!(
        authorize(json!({ "resource": format!("{RESOURCE}/messages") })),
        Some(McpResourceRewrite::Query(map(
            json!({ "resource": RESOURCE })
        )))
    );
}

#[test]
fn normalizes_the_legacy_endpoint_when_exchanging_the_authorization_code() {
    assert_eq!(
        token(json!({ "code": "x", "resource": format!("{RESOURCE}/sse") })),
        Some(McpResourceRewrite::Body(map(
            json!({ "code": "x", "resource": RESOURCE })
        )))
    );
}

#[test]
fn keeps_a_resource_the_client_supplied() {
    assert_eq!(authorize(json!({ "resource": "https://other/mcp" })), None);
}

#[test]
fn keeps_a_token_resource_the_client_supplied() {
    assert_eq!(token(json!({ "resource": "https://other/mcp" })), None);
}

#[test]
fn keeps_multiple_requested_resources_untouched() {
    assert_eq!(
        authorize(json!({ "resource": ["https://other/mcp"] })),
        None
    );
}

#[test]
fn ignores_unrelated_paths() {
    assert_eq!(
        default_mcp_resource("/sign-in/email", None, None, RESOURCE),
        None
    );
}
