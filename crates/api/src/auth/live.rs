//! The production services of the workspace-token middleware: SQLite
//! (`WorkspaceApiToken`, `Workspace`), the in-memory rate counters of `crates/kv`.
//!
//! The quota services are the business port (P3.6+); until then the owner is
//! never blocked. Mobile access tokens (P3.5e) and MCP OAuth tokens (P3.5e)
//! are refused as invalid tokens.

use std::sync::Arc;

use chatbotx_db::repo::models::{Workspace, WorkspaceApiToken};
use chatbotx_db::repo::{find_one, workspace_api_token};
use chatbotx_db::{Db, DbError};
use chatbotx_kv::rate_limit::{ApiRateLimitInput, RateLimited, assert_api_not_rate_limited};
use chatbotx_kv::{Clock, Kv};
use serde_json::Value;

use super::{
    AuthWorkspace, McpAccessToken, McpTokenVerificationUnavailable, MobileWorkspacePrincipal,
    OAuthPrincipalResolution, OwnerAccessState, RequestApiToken, WorkspaceQuota,
    WorkspaceTokenDeps, WorkspaceTokenLookup, invalid_token_error,
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
    async fn get_access_state(&self, _owner_id: &str) -> OwnerAccessState {
        OwnerAccessState::default()
    }

    async fn is_at_limit(&self, _user_id: &str, _metric: &str) -> bool {
        false
    }
}

impl WorkspaceTokenDeps for LiveAuthDeps {
    async fn find_workspace_by_token_hash(
        &self,
        token_hash: &str,
    ) -> Result<Option<WorkspaceTokenLookup>, ChatbotXException> {
        let token_hash = token_hash.to_string();
        let found = self
            .db
            .read(move |conn| {
                let repo_error =
                    |error: chatbotx_db::repo::Error| DbError::other(error.to_string());
                let Some(token) = workspace_api_token::find_by_token_hash(conn, &token_hash)
                    .map_err(repo_error)?
                else {
                    return Ok(None);
                };
                let workspace = find_one::<Workspace>(conn, "\"id\" = ?1", &[&token.workspace_id])
                    .map_err(repo_error)?;
                Ok(Some((token, workspace)))
            })
            .await
            .map_err(|error| ChatbotXException::new(error.to_string()))?;
        match found {
            None => Ok(None),
            Some((_, None)) => Err(ChatbotXException::with_code(
                "Workspace not found",
                "notFound",
                404,
            )),
            Some((token, Some(workspace))) => Ok(Some(lookup_of(&token, &workspace))),
        }
    }

    async fn assert_not_rate_limited(
        &self,
        scope: &str,
        key: &str,
        limit: Option<u64>,
    ) -> Result<(), RateLimited> {
        assert_api_not_rate_limited(
            ApiRateLimitInput {
                scope,
                key,
                store: &self.kv.mem,
                now_ms: self.clock.now_ms(),
                limit,
            },
            None,
        )
    }

    fn warn(&self, fields: Value, message: &str) {
        tracing::warn!(fields = %fields, "{message}");
    }

    fn is_cloud(&self) -> bool {
        self.is_cloud
    }

    fn mobile_api_audience(&self) -> Option<String> {
        None
    }

    async fn resolve_mobile_principal(
        &self,
        bearer_token: &str,
        requested_workspace_id: Option<&str>,
        method: Option<&str>,
        path: Option<&str>,
    ) -> Result<MobileWorkspacePrincipal, OrpcError> {
        let _ = (bearer_token, requested_workspace_id, method, path);
        Err(invalid_token_error())
    }

    async fn verify_mcp_access_token(
        &self,
        bearer_token: &str,
    ) -> Result<Option<McpAccessToken>, McpTokenVerificationUnavailable> {
        let _ = bearer_token;
        Ok(None)
    }

    async fn resolve_oauth_principal(
        &self,
        client_id: &str,
        user_id: &str,
        workspace_id: Option<&str>,
    ) -> OAuthPrincipalResolution {
        let _ = (client_id, user_id, workspace_id);
        OAuthPrincipalResolution::InvalidToken
    }

    fn now_ms(&self) -> i64 {
        self.clock.now_ms()
    }
}

/// Scopes of a token row: `None` (all scopes) or the listed names.
fn token_scopes(token: &WorkspaceApiToken) -> Option<Vec<String>> {
    let scopes = token.scopes.as_ref().filter(|scopes| !scopes.0.is_null())?;
    Some(
        scopes
            .0
            .as_array()
            .map(|names| {
                names
                    .iter()
                    .filter_map(|name| name.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default(),
    )
}

fn lookup_of(token: &WorkspaceApiToken, workspace: &Workspace) -> WorkspaceTokenLookup {
    WorkspaceTokenLookup {
        workspace: AuthWorkspace {
            id: workspace.id.to_string(),
            name: workspace.name.clone(),
            owner_id: workspace.owner_id.to_string(),
            scheduled_deletion_at: workspace.scheduled_deletion_at,
        },
        api_token: RequestApiToken {
            id: token.id.to_string(),
            workspace_id: token.workspace_id.to_string(),
            permission: token.permission.clone(),
            scopes: token_scopes(token),
            is_default: token.is_default,
        },
    }
}
