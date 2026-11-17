//! `packages/business/src/mcp-oauth/service.ts` (`mcpOAuthService`, Q.A1):
//! the workspace an MCP OAuth grant acts on, the consent-screen workspaces,
//! the consent binding and the grant list / revoke / purge.
//!
//! ponytail: support-access sessions are not ported (`resolveWorkspaceAccess`
//! = the real membership only), so a synthetic support membership never
//! exists here.

use chatbotx_db::Db;
use chatbotx_db::repo::{
    mobile_refresh_token, oauth_access_token, oauth_client, oauth_consent, oauth_refresh_token,
};
use chatbotx_db::rusqlite::{Connection, OptionalExtension, params};
use serde_json::Value;

use crate::workspace_member::is_workspace_admin_member;
use crate::{BusinessError, Result, tx};

/// The workspace of a principal (`WorkspaceModel` fields the callers read).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrantWorkspace {
    pub id: i64,
    pub name: String,
    pub owner_id: i64,
    pub scheduled_deletion_at: Option<i64>,
}

/// `McpOAuthResolution`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution {
    Principal {
        workspace: GrantWorkspace,
        user_id: i64,
        client_id: String,
    },
    InvalidToken,
    Forbidden,
    WorkspaceSelectionRequired(Vec<(i64, String)>),
}

/// `McpOAuthGrant`.
#[derive(Debug, Clone, PartialEq)]
pub struct Grant {
    pub client_id: String,
    pub client_name: String,
    pub client_icon: Option<String>,
    pub scopes: Vec<String>,
    pub first_authorized_at: i64,
    pub last_refreshed_at: Option<i64>,
    pub active_until: Option<i64>,
}

fn user_exists(c: &Connection, user_id: i64) -> Result<bool> {
    Ok(
        c.query_row(r#"SELECT 1 FROM "User" WHERE "id" = ?1"#, [user_id], |_| {
            Ok(())
        })
        .optional()?
        .is_some(),
    )
}

/// `findAdminWorkspace`: the workspace when the user is a real admin member
/// (uncached read).
fn find_admin_workspace(
    c: &Connection,
    user_id: i64,
    workspace_id: i64,
) -> Result<Option<GrantWorkspace>> {
    let row = c
        .query_row(
            r#"SELECT m."role", m."permissions", w."id", w."name", w."ownerId", w."scheduledDeletionAt"
               FROM "WorkspaceMember" m JOIN "Workspace" w ON w."id" = m."workspaceId"
               WHERE m."workspaceId" = ?1 AND m."userId" = ?2"#,
            params![workspace_id, user_id],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    GrantWorkspace {
                        id: r.get(2)?,
                        name: r.get(3)?,
                        owner_id: r.get(4)?,
                        scheduled_deletion_at: r.get(5)?,
                    },
                ))
            },
        )
        .optional()?;
    Ok(row.and_then(|(role, permissions, workspace)| {
        let permissions: Value = permissions
            .and_then(|p| serde_json::from_str(&p).ok())
            .unwrap_or(Value::Null);
        is_workspace_admin_member(&role, &permissions).then_some(workspace)
    }))
}

/// `listAuthorizableWorkspaces` on a connection.
fn authorizable_in(c: &Connection, user_id: i64) -> Result<Vec<(i64, String)>> {
    let mut stmt = c.prepare(
        r#"SELECT m."role", m."permissions", w."id", w."name"
           FROM "WorkspaceMember" m JOIN "Workspace" w ON w."id" = m."workspaceId"
           WHERE m."userId" = ?1 AND w."scheduledDeletionAt" IS NULL
           ORDER BY m."createdAt", m."id""#,
    )?;
    let rows = stmt
        .query_map([user_id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, String>(3)?,
            ))
        })?
        .collect::<chatbotx_db::rusqlite::Result<Vec<_>>>()?;
    Ok(rows
        .into_iter()
        .filter(|(role, permissions, _, _)| {
            let permissions: Value = permissions
                .as_deref()
                .and_then(|p| serde_json::from_str(p).ok())
                .unwrap_or(Value::Null);
            is_workspace_admin_member(role, &permissions)
        })
        .map(|(_, _, id, name)| (id, name))
        .collect())
}

/// `revokeGrant` on a write connection: consents first, then refresh and
/// access tokens; the deleted count.
fn revoke_in(c: &Connection, user_id: i64, client_id: &str) -> Result<usize> {
    let consents = oauth_consent::delete_by_user_and_client(c, user_id, client_id)?;
    let refresh = oauth_refresh_token::delete_by_user_and_client(c, user_id, client_id)?;
    let access = oauth_access_token::delete_by_user_and_client(c, user_id, client_id)?;
    Ok(consents + refresh + access)
}

/// `resolvePrincipal({ clientId, userId, workspaceId })`.
pub async fn resolve_principal(
    db: &Db,
    client_id: &str,
    user_id: i64,
    workspace_id: Option<i64>,
) -> Result<Resolution> {
    let client_id = client_id.to_string();
    tx::write(db, move |c| {
        let client = oauth_client::find_by_client_id(c, &client_id)?;
        if client.is_none_or(|client| client.disabled) || !user_exists(c, user_id)? {
            return Ok(Resolution::InvalidToken);
        }
        let Some(consented) =
            oauth_consent::find_latest_given_by_user_and_client(c, user_id, &client_id)?
        else {
            return Ok(Resolution::InvalidToken);
        };
        if let (Some(consented), Some(requested)) = (consented, workspace_id)
            && consented != requested
        {
            return Ok(Resolution::Forbidden);
        }
        let target = match consented.or(workspace_id) {
            Some(id) => id,
            None => {
                let workspaces = authorizable_in(c, user_id)?;
                match workspaces.as_slice() {
                    [] => return Ok(Resolution::Forbidden),
                    [(id, _)] => *id,
                    _ => return Ok(Resolution::WorkspaceSelectionRequired(workspaces)),
                }
            }
        };
        let Some(workspace) = find_admin_workspace(c, user_id, target)? else {
            if consented.is_some() {
                let deleted = revoke_in(c, user_id, &client_id)?;
                if deleted > 0 {
                    tracing::info!(user_id, client_id, deleted, "Revoked MCP OAuth grant");
                }
            }
            return Ok(Resolution::Forbidden);
        };
        Ok(Resolution::Principal {
            workspace,
            user_id,
            client_id,
        })
    })
    .await
}

/// `listAuthorizableWorkspaces`: admin workspaces not scheduled for deletion.
pub async fn list_authorizable_workspaces(db: &Db, user_id: i64) -> Result<Vec<(i64, String)>> {
    tx::read(db, move |c| authorizable_in(c, user_id)).await
}

fn not_admin() -> BusinessError {
    BusinessError::exception(
        "You must be an admin of the selected workspace",
        "forbidden",
        403,
    )
}

/// `assertAuthorizableWorkspace`.
pub async fn assert_authorizable_workspace(
    db: &Db,
    user_id: i64,
    workspace_id: i64,
) -> Result<GrantWorkspace> {
    tx::read(db, move |c| {
        find_admin_workspace(c, user_id, workspace_id)?
            .filter(|w| w.scheduled_deletion_at.is_none())
            .ok_or_else(not_admin)
    })
    .await
}

/// `bindConsentWorkspace`.
pub async fn bind_consent_workspace(
    db: &Db,
    user_id: i64,
    client_id: &str,
    workspace_id: i64,
) -> Result<()> {
    let client_id = client_id.to_string();
    tx::write(db, move |c| {
        let workspace = find_admin_workspace(c, user_id, workspace_id)?
            .filter(|w| w.scheduled_deletion_at.is_none())
            .ok_or_else(not_admin)?;
        let updated =
            oauth_consent::set_workspace_by_user_and_client(c, user_id, &client_id, workspace.id)?;
        if updated == 0 {
            return Err(BusinessError::exception(
                "OAuth consent not found",
                "notFound",
                404,
            ));
        }
        tracing::info!(
            user_id,
            client_id,
            workspace_id = workspace.id,
            "Bound MCP OAuth consent to workspace"
        );
        Ok(())
    })
    .await
}

/// `listGrantsForUser`.
pub async fn list_grants_for_user(db: &Db, user_id: i64, now_ms: i64) -> Result<Vec<Grant>> {
    let rows = tx::read(db, move |c| {
        Ok(oauth_consent::list_given_by_user(c, user_id, now_ms)?)
    })
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let mut scopes: Vec<String> = Vec::new();
            for scope in row.scopes.0.as_array().into_iter().flatten() {
                if let Some(scope) = scope.as_str()
                    && !scopes.iter().any(|s| s == scope)
                {
                    scopes.push(scope.to_string());
                }
            }
            Grant {
                client_name: row.client_name.unwrap_or_else(|| row.client_id.clone()),
                client_id: row.client_id,
                client_icon: row.client_icon,
                scopes,
                first_authorized_at: row.first_authorized_at,
                last_refreshed_at: row.last_refreshed_at,
                active_until: row.active_until,
            }
        })
        .collect())
}

/// `revokeGrant`: `true` when anything was deleted.
pub async fn revoke_grant(db: &Db, user_id: i64, client_id: &str) -> Result<bool> {
    let client_id = client_id.to_string();
    tx::write(db, move |c| {
        let deleted = revoke_in(c, user_id, &client_id)?;
        if deleted > 0 {
            tracing::info!(user_id, client_id, deleted, "Revoked MCP OAuth grant");
        }
        Ok(deleted > 0)
    })
    .await
}

/// `purgeExpiredTokens`: expired access, OAuth refresh and mobile refresh tokens.
pub async fn purge_expired_tokens(db: &Db, now_ms: i64) -> Result<usize> {
    tx::write(db, move |c| {
        Ok(oauth_access_token::delete_expired(c, now_ms)?
            + oauth_refresh_token::delete_expired(c, now_ms)?
            + mobile_refresh_token::delete_expired(c, now_ms)?)
    })
    .await
}
