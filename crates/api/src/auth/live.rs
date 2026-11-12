//! The production services of the workspace-token middleware: SQLite
//! (`WorkspaceApiToken`, `Workspace`), the in-memory rate counters of `crates/kv`.
//!
//! The quota services are the business port (P3.6+); until then the owner is
//! never blocked. Mobile access tokens (P3.5e) and MCP OAuth tokens (P3.5e)
//! are refused as invalid tokens.

use std::sync::Arc;

use chatbotx_db::Db;
use chatbotx_kv::rate_limit::RateLimited;
use chatbotx_kv::{Clock, Kv};
use serde_json::Value;

use super::{
    McpAccessToken, McpTokenVerificationUnavailable, MobileWorkspacePrincipal,
    OAuthPrincipalResolution, OwnerAccessState, WorkspaceQuota, WorkspaceTokenDeps,
    WorkspaceTokenLookup,
};
use crate::errors::ChatbotXException;
use crate::orpc::OrpcError;

/// The services of the running server.
#[derive(Clone)]
pub struct LiveAuthDeps {
    pub db: Arc<Db>,
    pub kv: Arc<Kv>,
    pub clock: Arc<dyn Clock>,
    /// `isCloud()` (`CHATBOTX_DEPLOYMENT=cloud`).
    pub is_cloud: bool,
}

impl WorkspaceQuota for LiveAuthDeps {
    async fn get_access_state(&self, owner_id: &str) -> OwnerAccessState {
        let _ = owner_id;
        todo!("P3.5b")
    }

    async fn is_at_limit(&self, user_id: &str, metric: &str) -> bool {
        let _ = (user_id, metric);
        todo!("P3.5b")
    }
}

impl WorkspaceTokenDeps for LiveAuthDeps {
    async fn find_workspace_by_token_hash(
        &self,
        token_hash: &str,
    ) -> Result<Option<WorkspaceTokenLookup>, ChatbotXException> {
        let _ = token_hash;
        todo!("P3.5b")
    }

    async fn assert_not_rate_limited(
        &self,
        scope: &str,
        key: &str,
        limit: Option<u64>,
    ) -> Result<(), RateLimited> {
        let _ = (scope, key, limit);
        todo!("P3.5b")
    }

    fn warn(&self, fields: Value, message: &str) {
        let _ = (fields, message);
        todo!("P3.5b")
    }

    fn is_cloud(&self) -> bool {
        self.is_cloud
    }

    fn mobile_api_audience(&self) -> Option<String> {
        todo!("P3.5b")
    }

    async fn resolve_mobile_principal(
        &self,
        bearer_token: &str,
        requested_workspace_id: Option<&str>,
        method: Option<&str>,
        path: Option<&str>,
    ) -> Result<MobileWorkspacePrincipal, OrpcError> {
        let _ = (bearer_token, requested_workspace_id, method, path);
        todo!("P3.5b")
    }

    async fn verify_mcp_access_token(
        &self,
        bearer_token: &str,
    ) -> Result<Option<McpAccessToken>, McpTokenVerificationUnavailable> {
        let _ = bearer_token;
        todo!("P3.5b")
    }

    async fn resolve_oauth_principal(
        &self,
        client_id: &str,
        user_id: &str,
        workspace_id: Option<&str>,
    ) -> OAuthPrincipalResolution {
        let _ = (client_id, user_id, workspace_id);
        todo!("P3.5b")
    }

    fn now_ms(&self) -> i64 {
        self.clock.now_ms()
    }
}
