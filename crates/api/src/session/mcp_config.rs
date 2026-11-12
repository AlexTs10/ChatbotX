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
    let _ = resource;
    todo!("P3.5e")
}

/// `defaultMcpResource({ path, query, body }, resource)`.
pub fn default_mcp_resource(
    path: &str,
    query: Option<&Map<String, Value>>,
    body: Option<&Map<String, Value>>,
    resource: &str,
) -> Option<McpResourceRewrite> {
    let _ = (path, query, body, resource);
    todo!("P3.5e")
}
