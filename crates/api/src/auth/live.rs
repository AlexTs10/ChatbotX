//! The production services of the workspace-token middleware: SQLite
//! (`WorkspaceApiToken`, `Workspace`), the in-memory rate counters of `crates/kv`.
//!
//! The quota services are the business port (P3.6+); until then the owner is
//! never blocked. MCP OAuth tokens and mobile access tokens are verified (Q.A1;
//! mobile members only on the allowlisted, member-scoped routes).

use std::sync::Arc;

use chatbotx_db::repo::models::{Workspace, WorkspaceApiToken};
use chatbotx_db::repo::{find_one, workspace_api_token};
use chatbotx_db::{Db, DbError};
use chatbotx_kv::rate_limit::{ApiRateLimitInput, RateLimited, assert_api_not_rate_limited};
use chatbotx_kv::{Clock, Kv};
use serde_json::Value;

use super::{
    AuthWorkspace, ChannelApiLookup, McpAccessToken, McpTokenVerificationUnavailable,
    MobileWorkspacePrincipal, OAuthPrincipalResolution, OwnerAccessState, RequestApiToken,
    WorkspaceQuota, WorkspaceTokenDeps, WorkspaceTokenLookup,
};
use crate::errors::ChatbotXException;
use crate::orpc::OrpcError;

/// The services of the running server.
#[derive(Clone)]
pub struct LiveAuthDeps {
    pub db: Arc<Db>,
    pub kv: Arc<Kv>,
    pub clock: Arc<dyn Clock>,
    /// better-auth `baseURL` (`<BETTER_AUTH_URL>/api/auth`): the MCP token issuer.
    pub auth_base_url: String,
    /// The MCP resource URL (the MCP token audience).
    pub mcp_resource_url: String,
    /// `isCloud()` (`CHATBOTX_DEPLOYMENT=cloud`).
    pub is_cloud: bool,
    /// `getMobileApiAudience()`; `None` = mobile access tokens are off on `/v1`.
    pub mobile_audience: Option<String>,
}

/// `INBOX_SCOPES` of `deriveMemberScopes`.
const INBOX_SCOPES: [&str; 5] = ["contacts", "inbox", "automation", "broadcasts", "media"];

fn invalid_mobile_token_error() -> OrpcError {
    OrpcError::new(
        "INVALID_CHATBOT_TOKEN",
        Some(401),
        Some("Invalid or missing access token"),
    )
}

/// `isValidWorkspaceId`: 1-20 digits within the i64 range.
fn valid_workspace_id(value: &str) -> Option<i64> {
    (!value.is_empty() && value.len() <= 20 && value.bytes().all(|b| b.is_ascii_digit()))
        .then(|| value.parse::<i64>().ok())
        .flatten()
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
        self.mobile_audience.clone()
    }

    /// `authenticateMobileBearer`: verify the JWT, then load its user.
    async fn authenticate_mobile_bearer(
        &self,
        bearer_token: &str,
    ) -> Result<Option<super::MobileUser>, OrpcError> {
        let Some(audience) = self.mobile_audience.as_deref() else {
            return Ok(None);
        };
        let verifier = crate::session::DbJwtVerifier {
            db: &self.db,
            now_ms: self.clock.now_ms(),
        };
        let subject = match crate::session::verify_mobile_access_token(
            &verifier,
            &self.auth_base_url,
            audience,
            bearer_token,
        )
        .await
        {
            Ok(subject) => subject,
            Err(error) => {
                if let crate::session::TokenVerifyError::Rethrown(error) = &error {
                    tracing::error!(error = ?error, "mobile access token verification failed");
                }
                return Err(OrpcError::new(
                    "SERVICE_UNAVAILABLE",
                    None,
                    Some("Token verification is temporarily unavailable"),
                ));
            }
        };
        let Some(subject) = subject else {
            return Ok(None);
        };
        let Ok(user_id) = subject.user_id.parse::<i64>() else {
            return Ok(None);
        };
        let device_id = subject.device_id;
        self.db
            .read(move |conn| {
                use chatbotx_db::rusqlite::OptionalExtension as _;
                conn.query_row(
                    r#"SELECT "name", "email", "image", "mustChangePassword" FROM "User" WHERE "id" = ?1"#,
                    [user_id],
                    |row| {
                        Ok(super::MobileUser {
                            id: user_id,
                            name: row.get(0)?,
                            email: row.get(1)?,
                            image: row.get(2)?,
                            must_change_password: row.get(3)?,
                            device_id,
                        })
                    },
                )
                .optional()
                .map_err(|e| DbError::other(e.to_string()))
            })
            .await
            .map_err(|error| {
                tracing::error!(%error, "mobile user lookup failed");
                OrpcError::new("INTERNAL_SERVER_ERROR", Some(500), None)
            })
    }

    /// `resolveMobileWorkspacePrincipal`: the verified user, refused while a
    /// password change is pending and on every route outside the member
    /// allowlist, then the membership of `x-workspace-id`.
    async fn resolve_mobile_principal(
        &self,
        bearer_token: &str,
        requested_workspace_id: Option<&str>,
        method: Option<&str>,
        path: Option<&str>,
    ) -> Result<MobileWorkspacePrincipal, OrpcError> {
        let Some(user) = self.authenticate_mobile_bearer(bearer_token).await? else {
            return Err(invalid_mobile_token_error());
        };
        let user_id = user.id;
        let db_error = |error: DbError| {
            tracing::error!(%error, "mobile principal lookup failed");
            OrpcError::new("INTERNAL_SERVER_ERROR", Some(500), None)
        };
        if user.must_change_password {
            return Err(OrpcError::new(
                "FORBIDDEN",
                None,
                Some("Password change required"),
            ));
        }
        if !super::is_mobile_member_route(method, path) {
            return Err(OrpcError::new(
                "FORBIDDEN",
                None,
                Some("This operation is not available to mobile sessions"),
            ));
        }
        let Some(workspace_id) = requested_workspace_id.and_then(valid_workspace_id) else {
            return Err(OrpcError::new(
                "WORKSPACE_SELECTION_REQUIRED",
                Some(400),
                Some("A numeric x-workspace-id header is required for this request"),
            ));
        };
        let membership = self
            .db
            .read(move |conn| {
                use chatbotx_db::repo::models::WorkspaceMember;
                let error = |e: chatbotx_db::repo::Error| DbError::other(e.to_string());
                let member = find_one::<WorkspaceMember>(
                    conn,
                    r#"WHERE "workspaceId" = ?1 AND "userId" = ?2"#,
                    &[&workspace_id, &user_id],
                )
                .map_err(error)?;
                let Some(member) = member else {
                    return Ok(None);
                };
                let workspace = find_one::<Workspace>(conn, r#"WHERE "id" = ?1"#, &[&workspace_id])
                    .map_err(error)?;
                Ok(workspace.map(|w| (member, w)))
            })
            .await
            .map_err(db_error)?;
        let Some((member, workspace)) = membership else {
            return Err(OrpcError::new(
                "FORBIDDEN",
                None,
                Some("Account has no access to this workspace"),
            ));
        };
        let permissions = member.permissions.0.clone();
        let conversation = chatbotx_business::member_scope::resolve_conversation_scope(
            &self.db,
            workspace_id,
            user_id,
            &permissions,
        )
        .await
        .map_err(|error| {
            tracing::error!(error = %error.message(), "mobile member scope lookup failed");
            OrpcError::new("INTERNAL_SERVER_ERROR", Some(500), None)
        })?;
        let scopes = if chatbotx_business::workspace_member::has_contacts_access(&permissions) {
            INBOX_SCOPES.map(String::from).to_vec()
        } else {
            Vec::new()
        };
        let device_id = user.device_id;
        // `mobileCredentialId({ user, deviceId, workspace })`.
        let credential_id = format!(
            "mobile:{workspace_id}:{user_id}:{}",
            device_id.as_deref().unwrap_or("-")
        );
        Ok(MobileWorkspacePrincipal {
            workspace: AuthWorkspace {
                id: workspace.id.to_string(),
                name: workspace.name,
                owner_id: workspace.owner_id.to_string(),
                scheduled_deletion_at: workspace.scheduled_deletion_at,
            },
            api_token: RequestApiToken {
                id: credential_id.clone(),
                workspace_id: workspace_id.to_string(),
                permission: "full".into(),
                scopes: Some(scopes),
                is_default: false,
            },
            user_id: user_id.to_string(),
            member_id: Some(member.id.to_string()),
            device_id,
            credential_id,
            member: Arc::new(super::MobileMember {
                user_id,
                permissions,
                conversation,
            }),
        })
    }

    async fn verify_mcp_access_token(
        &self,
        bearer_token: &str,
    ) -> Result<Option<McpAccessToken>, McpTokenVerificationUnavailable> {
        let verifier = crate::session::DbJwtVerifier {
            db: &self.db,
            now_ms: self.clock.now_ms(),
        };
        match crate::session::verify_mcp_access_token(
            &verifier,
            &self.auth_base_url,
            &self.mcp_resource_url,
            bearer_token,
        )
        .await
        {
            Ok(subject) => Ok(subject.map(|s| McpAccessToken {
                client_id: s.client_id,
                user_id: s.user_id,
            })),
            Err(crate::session::TokenVerifyError::Unavailable) => {
                Err(McpTokenVerificationUnavailable)
            }
            // A rethrown verifier error: the token is not accepted.
            Err(crate::session::TokenVerifyError::Rethrown(error)) => {
                tracing::error!(error = ?error, "MCP access token verification failed");
                Err(McpTokenVerificationUnavailable)
            }
        }
    }

    async fn resolve_oauth_principal(
        &self,
        client_id: &str,
        user_id: &str,
        workspace_id: Option<&str>,
    ) -> OAuthPrincipalResolution {
        use chatbotx_business::mcp_oauth::{Resolution, resolve_principal};
        let Ok(user_id) = user_id.parse::<i64>() else {
            return OAuthPrincipalResolution::InvalidToken;
        };
        let workspace_id = match workspace_id.map(str::parse::<i64>) {
            None => None,
            Some(Ok(id)) => Some(id),
            // A non-numeric workspace id is no workspace of the grant.
            Some(Err(_)) => return OAuthPrincipalResolution::Forbidden,
        };
        match resolve_principal(&self.db, client_id, user_id, workspace_id).await {
            Ok(Resolution::Principal {
                workspace,
                user_id,
                client_id,
            }) => OAuthPrincipalResolution::Principal {
                workspace: AuthWorkspace {
                    id: workspace.id.to_string(),
                    name: workspace.name,
                    owner_id: workspace.owner_id.to_string(),
                    scheduled_deletion_at: workspace.scheduled_deletion_at,
                },
                user_id: user_id.to_string(),
                client_id,
            },
            Ok(Resolution::InvalidToken) => OAuthPrincipalResolution::InvalidToken,
            Ok(Resolution::Forbidden) => OAuthPrincipalResolution::Forbidden,
            Ok(Resolution::WorkspaceSelectionRequired(choices)) => {
                OAuthPrincipalResolution::WorkspaceSelectionRequired(
                    choices
                        .into_iter()
                        .map(|(id, name)| super::WorkspaceChoice {
                            id: id.to_string(),
                            name,
                        })
                        .collect(),
                )
            }
            Err(error) => {
                tracing::error!(error = %error.message(), "MCP OAuth principal lookup failed");
                OAuthPrincipalResolution::InvalidToken
            }
        }
    }

    fn now_ms(&self) -> i64 {
        self.clock.now_ms()
    }

    async fn find_channel_api_by_token_hash(
        &self,
        token_hash: &str,
    ) -> Result<Option<ChannelApiLookup>, ChatbotXException> {
        let token_hash = token_hash.to_string();
        self.db
            .read(move |conn| find_channel_api(conn, &token_hash))
            .await
            .map_err(|error| ChatbotXException::new(error.to_string()))
    }
}

/// `integrationApiRepository.findByTokenHash` + workspace + inbox (private
/// query: no `IntegrationApi` repository in `crates/db` yet).
fn find_channel_api(
    conn: &chatbotx_db::rusqlite::Connection,
    token_hash: &str,
) -> Result<Option<ChannelApiLookup>, DbError> {
    use chatbotx_db::rusqlite::OptionalExtension as _;
    let row = conn
        .query_row(
            r#"SELECT a."id", a."enabled", a."inboxId", w."id", w."name", w."ownerId",
                      w."scheduledDeletionAt", i."id"
               FROM "IntegrationApi" a
               JOIN "Workspace" w ON w."id" = a."workspaceId"
               LEFT JOIN "Inbox" i ON i."id" = a."inboxId"
               WHERE a."tokenHash" = ?1 LIMIT 1"#,
            [token_hash],
            |row| {
                Ok(ChannelApiLookup {
                    integration_api_id: row.get::<_, i64>(0)?.to_string(),
                    enabled: row.get::<_, Option<bool>>(1)?.unwrap_or(false),
                    workspace: AuthWorkspace {
                        id: row.get::<_, i64>(3)?.to_string(),
                        name: row.get(4)?,
                        owner_id: row.get::<_, i64>(5)?.to_string(),
                        scheduled_deletion_at: row.get(6)?,
                    },
                    inbox_id: row.get::<_, Option<i64>>(7)?.map(|id| id.to_string()),
                })
            },
        )
        .optional()?;
    Ok(row)
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
