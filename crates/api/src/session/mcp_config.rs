//! `packages/auth/src/mcp-config.ts`: the MCP OAuth plugin options and the
//! `defaultMcpResource` request rewrite (fills in / canonicalizes `resource`
//! on `/oauth2/authorize` and `/oauth2/token`).

use std::collections::BTreeSet;

use serde_json::{Map, Value};

/// `MCP_OAUTH_SCOPES`.
pub const MCP_OAUTH_SCOPES: &[&str] = &["openid", "profile", "email", "offline_access"];
/// Access token lifetime (5 minutes).
pub const MCP_ACCESS_TOKEN_TTL_SECONDS: u64 = 300;
/// Refresh token lifetime (30 days).
pub const MCP_REFRESH_TOKEN_TTL_SECONDS: u64 = 2_592_000;

/// `buildMcpPluginOptions(resource)` (the fields the Rust OAuth server reads).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpPluginOptions {
    pub access_token_expires_in: u64,
    pub refresh_token_expires_in: u64,
    pub allow_dynamic_client_registration: bool,
    pub allow_unauthenticated_client_registration: bool,
    pub client_registration_default_resources: Vec<String>,
    pub client_registration_default_scopes: Vec<String>,
    pub consent_page: &'static str,
    pub login_page: &'static str,
    pub grant_types: Vec<&'static str>,
    pub resource: String,
    pub cached_resources: BTreeSet<String>,
}

/// The rewrite `defaultMcpResource` asks better-auth to apply.
#[derive(Debug, Clone, PartialEq)]
pub enum McpResourceRewrite {
    /// `{ context: { query } }`.
    Query(Map<String, Value>),
    /// `{ context: { body } }`.
    Body(Map<String, Value>),
}

/// `buildMcpPluginOptions(resource)`.
pub fn build_mcp_plugin_options(resource: &str) -> McpPluginOptions {
    McpPluginOptions {
        access_token_expires_in: MCP_ACCESS_TOKEN_TTL_SECONDS,
        refresh_token_expires_in: MCP_REFRESH_TOKEN_TTL_SECONDS,
        allow_dynamic_client_registration: true,
        allow_unauthenticated_client_registration: true,
        client_registration_default_resources: vec![resource.to_string()],
        client_registration_default_scopes: MCP_OAUTH_SCOPES
            .iter()
            .map(|scope| (*scope).to_string())
            .collect(),
        consent_page: "/oauth/consent",
        login_page: "/oauth/sign-in",
        grant_types: vec!["authorization_code", "refresh_token"],
        resource: resource.to_string(),
        cached_resources: BTreeSet::from([resource.to_string()]),
    }
}

/// Legacy MCP transport endpoints a client may name as the resource.
const LEGACY_MCP_TRANSPORT_PATHS: &[&str] = &["/messages", "/sse"];

/// `hasResource(params)`: a non-empty array or a truthy value.
fn has_resource(params: Option<&Map<String, Value>>) -> bool {
    match params.and_then(|params| params.get("resource")) {
        None | Some(Value::Null | Value::Bool(false)) => false,
        Some(Value::Array(values)) => !values.is_empty(),
        Some(Value::String(text)) => !text.is_empty(),
        Some(Value::Number(number)) => number.as_f64().is_some_and(|n| n != 0.0),
        Some(_) => true,
    }
}

/// `withCanonicalMcpResource(params, resource)`.
fn with_canonical_mcp_resource(
    params: Option<&Map<String, Value>>,
    resource: &str,
) -> Option<Map<String, Value>> {
    let canonical = if has_resource(params) {
        let requested = params.and_then(|params| params.get("resource"));
        let legacy = requested.and_then(Value::as_str).is_some_and(|requested| {
            LEGACY_MCP_TRANSPORT_PATHS
                .iter()
                .any(|path| requested == format!("{resource}{path}"))
        });
        legacy.then_some(resource)
    } else {
        Some(resource)
    }?;
    let mut rewritten = params.cloned().unwrap_or_default();
    rewritten.insert("resource".to_string(), Value::String(canonical.to_string()));
    Some(rewritten)
}

/// `defaultMcpResource({ path, query, body }, resource)`.
pub fn default_mcp_resource(
    path: &str,
    query: Option<&Map<String, Value>>,
    body: Option<&Map<String, Value>>,
    resource: &str,
) -> Option<McpResourceRewrite> {
    match path {
        "/oauth2/authorize" => {
            with_canonical_mcp_resource(query, resource).map(McpResourceRewrite::Query)
        }
        "/oauth2/token" => {
            with_canonical_mcp_resource(body, resource).map(McpResourceRewrite::Body)
        }
        _ => None,
    }
}
