//! `invitation/service.ts`: `create` and `buildInviteUrl` (Q.D11 slice 2).

use base64::Engine as _;
use chatbotx_db::Db;
use chatbotx_db::repo::{new_id, now_ms};
use serde_json::Value;

use crate::{Result, tx};

/// `INVITATION_TTL_DAYS`: 1.
pub const INVITATION_TTL_MS: i64 = 24 * 60 * 60 * 1000;

/// `generateInvitationCode`: 16 CSPRNG bytes, base64url without padding.
pub fn generate_invitation_code() -> String {
    let mut bytes = [0u8; 16];
    ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut bytes)
        .expect("system random");
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// A new invitation.
#[derive(Debug, Clone, PartialEq)]
pub struct Invitation {
    pub id: i64,
    pub code: String,
    pub expires_at: i64,
}

/// `create`: a shareable code for the workspace, valid one day.
pub async fn create(
    db: &Db,
    workspace_id: i64,
    invited_by: i64,
    permissions: Value,
) -> Result<Invitation> {
    tx::write(db, move |tx| {
        let invitation = Invitation {
            id: new_id(),
            code: generate_invitation_code(),
            expires_at: now_ms() + INVITATION_TTL_MS,
        };
        tx.execute(
            r#"INSERT INTO "Invitation" ("id", "code", "permissions", "expiresAt", "workspaceId", "invitedBy")
               VALUES (?1, ?2, ?3, ?4, ?5, ?6)"#,
            chatbotx_db::rusqlite::params![
                invitation.id,
                invitation.code,
                permissions.to_string(),
                invitation.expires_at,
                workspace_id,
                invited_by
            ],
        )
        .map_err(chatbotx_db::repo::Error::from)?;
        Ok(invitation)
    })
    .await
}

/// `buildInviteUrl`: `<appUrl>/invitations/<code>` (one tenant: the app URL
/// serves every workspace).
pub fn build_invite_url(app_url: &str, code: &str) -> String {
    let encoded: String = code
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z'
            | b'a'..=b'z'
            | b'0'..=b'9'
            | b'-'
            | b'_'
            | b'.'
            | b'!'
            | b'~'
            | b'*'
            | b'\''
            | b'('
            | b')' => (b as char).to_string(),
            other => format!("%{other:02X}"),
        })
        .collect();
    format!("{}/invitations/{encoded}", app_url.trim_end_matches('/'))
}

// ---- lookup and accept (Q.F2 web run 7) -----------------------------------

/// `InvitationSummary` (`getInvitationDetails`).
#[derive(Debug, Clone, PartialEq)]
pub struct InvitationSummary {
    pub code: String,
    pub expires_at: i64,
    pub workspace_name: String,
    pub workspace_logo: Option<String>,
    pub scheduled_deletion_at: Option<i64>,
    pub inviter_name: Option<String>,
    pub inviter_image: Option<String>,
}

/// Why `lookupActive` rejected a code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Rejection {
    NotFound,
    Expired,
    /// `no_workspace` with (`true`) or without a `workspaceId` on the row.
    NoWorkspace(bool),
}

/// `lookupActive`. ponytail: one tenant (see [`build_invite_url`]), so the
/// `tenant_mismatch` check of the original never fires; add it with
/// white-label tenants.
fn lookup_active(
    conn: &chatbotx_db::rusqlite::Connection,
    code: &str,
    now: i64,
) -> chatbotx_db::repo::Result<
    std::result::Result<
        (
            chatbotx_db::repo::models::Invitation,
            chatbotx_db::repo::models::Workspace,
        ),
        Rejection,
    >,
> {
    use chatbotx_db::repo::Model as _;
    use chatbotx_db::repo::models::Invitation as Row;
    use chatbotx_db::rusqlite::OptionalExtension as _;
    let row = conn
        .query_row(
            &format!(r#"{} WHERE "code" = ?1"#, Row::select_sql()),
            [code],
            Row::from_row,
        )
        .optional()?;
    let Some(row) = row else {
        return Ok(Err(Rejection::NotFound));
    };
    let Some(workspace_id) = row.workspace_id.filter(|_| row.expires_at >= now) else {
        return Ok(Err(if row.expires_at < now {
            Rejection::Expired
        } else {
            Rejection::NoWorkspace(false)
        }));
    };
    let Some(workspace) = crate::workspace::find(conn, workspace_id)? else {
        return Ok(Err(Rejection::NoWorkspace(true)));
    };
    Ok(Ok((row, workspace)))
}

/// `getInvitationDetails`: `None` for any rejected code (logged at debug).
pub async fn details(db: &Db, code: String) -> Result<Option<InvitationSummary>> {
    tx::read(db, move |conn| {
        let (row, workspace) = match lookup_active(conn, &code, now_ms())? {
            Ok(found) => found,
            Err(reason) => {
                tracing::debug!(?reason, "Invitation lookup rejected");
                return Ok(None);
            }
        };
        let (inviter_name, inviter_image): (Option<String>, Option<String>) = conn
            .query_row(
                r#"SELECT "name", "image" FROM "User" WHERE "id" = ?1"#,
                [row.invited_by],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(chatbotx_db::repo::Error::from)?;
        Ok(Some(InvitationSummary {
            code: row.code,
            expires_at: row.expires_at,
            workspace_name: workspace.name,
            workspace_logo: workspace.logo,
            scheduled_deletion_at: workspace.scheduled_deletion_at,
            inviter_name,
            inviter_image,
        }))
    })
    .await
}

/// `acceptInvitationAction`: the code must be active; the user must not be a
/// member yet; the workspace must not be scheduled for deletion; the owner's
/// `teamMembers` quota must have room. Adds an `agent` member with the
/// invitation's permissions (every permission on community). Returns the
/// workspace id.
pub async fn accept(
    db: &Db,
    edition: crate::keys::Edition,
    code: String,
    user_id: i64,
) -> Result<i64> {
    use crate::BusinessError;
    tx::write(db, move |tx| {
        let (row, workspace) = match lookup_active(tx, &code, now_ms())? {
            Ok(found) => found,
            Err(Rejection::Expired) => {
                return Err(BusinessError::exception("Invitation expired", "systemError", 400));
            }
            Err(Rejection::NoWorkspace(false)) => {
                return Err(BusinessError::exception(
                    "Invalid invitation: no workspace associated",
                    "systemError",
                    400,
                ));
            }
            Err(_) => return Err(BusinessError::not_found("Invitation not found")),
        };
        let is_member: bool = tx
            .query_row(
                r#"SELECT EXISTS(SELECT 1 FROM "WorkspaceMember" WHERE "workspaceId" = ?1 AND "userId" = ?2)"#,
                [workspace.id, user_id],
                |r| r.get(0),
            )
            .map_err(chatbotx_db::repo::Error::from)?;
        if is_member {
            return Err(BusinessError::exception(
                "You are already a member of this workspace",
                "systemError",
                400,
            ));
        }
        if crate::workspace::is_scheduled_for_deletion(&workspace) {
            return Err(BusinessError::exception(
                "This workspace is no longer available.",
                "workspaceScheduledDeletion",
                403,
            ));
        }
        if crate::quota_enforcement::has_reached_limit(
            tx,
            workspace.owner_id,
            crate::user_quota::QuotaMetric::TeamMembers,
        )? {
            return Err(BusinessError::exception(
                "Team member limit reached for this workspace plan",
                "systemError",
                400,
            ));
        }
        // Defense in depth: community members always get every permission.
        let permissions = if edition.is_community() {
            crate::workspace_member::full_permissions()
        } else {
            row.permissions.0.clone()
        };
        let member_id =
            crate::workspace::insert_member(tx, workspace.id, user_id, "agent", &permissions)?;
        let notification_types = serde_json::json!({
            "notifyAdmin": true,
            "newMessageToHuman": true,
            "newOrder": crate::workspace_member::has_permission(&permissions, "ecommerce"),
        });
        tx.execute(
            r#"UPDATE "WorkspaceMember" SET "notificationTypes" = ?1 WHERE "id" = ?2"#,
            chatbotx_db::rusqlite::params![notification_types.to_string(), member_id],
        )
        .map_err(chatbotx_db::repo::Error::from)?;
        Ok(workspace.id)
    })
    .await
}
