//! `workspace-member/service.ts` (P3.6-X1): `listPaginated` and
//! `findByIdWithUser`, the reads of the public `/v1/members` routes.

use chatbotx_db::Db;
use chatbotx_db::repo::utils::{PaginationInput, get_pagination_with_defaults, like_contains};
use chatbotx_db::rusqlite::types::Value as Sql;
use chatbotx_db::rusqlite::{Row, params_from_iter};

use crate::{Result, tx};

/// One `WorkspaceMember` row with its user (`with: { user: true }`).
#[derive(Debug, Clone, PartialEq)]
pub struct MemberWithUser {
    pub id: i64,
    pub created_at: i64,
    pub updated_at: i64,
    pub workspace_id: i64,
    pub user_id: i64,
    pub role: String,
    /// The JSON columns as stored.
    pub notification_channels: String,
    pub notification_types: String,
    pub permissions: String,
    pub online_since: Option<i64>,
    pub user_name: Option<String>,
    pub user_image: Option<String>,
}

/// `{ data, pageCount }`.
#[derive(Debug, Clone, PartialEq)]
pub struct MemberPage {
    pub data: Vec<MemberWithUser>,
    pub page_count: i64,
}

const COLUMNS: &str = r#"m."id", m."createdAt", m."updatedAt", m."workspaceId", m."userId", m."role", m."notificationChannels", m."notificationTypes", m."permissions", m."onlineSince", u."name", u."image""#;

fn member(row: &Row<'_>) -> chatbotx_db::rusqlite::Result<MemberWithUser> {
    Ok(MemberWithUser {
        id: row.get(0)?,
        created_at: row.get(1)?,
        updated_at: row.get(2)?,
        workspace_id: row.get(3)?,
        user_id: row.get(4)?,
        role: row.get(5)?,
        notification_channels: row.get(6)?,
        notification_types: row.get(7)?,
        permissions: row.get(8)?,
        online_since: row.get(9)?,
        user_name: row.get(10)?,
        user_image: row.get(11)?,
    })
}

fn repo_err(error: chatbotx_db::rusqlite::Error) -> crate::BusinessError {
    crate::BusinessError::Repo(error.into())
}

/// `listPaginated`: the members of the workspace, `keyword` an ILIKE
/// substring of the user name; `pageCount = ceil(total / limit)`. The
/// original has no order (Postgres row order); this is `id` order.
pub async fn list_paginated(
    db: &Db,
    workspace_id: i64,
    page: Option<i64>,
    per_page: Option<i64>,
    keyword: Option<String>,
) -> Result<MemberPage> {
    tx::read(db, move |conn| {
        let pagination = get_pagination_with_defaults(PaginationInput { page, per_page });
        let mut filter = String::from(r#"m."workspaceId" = ?"#);
        let mut binds = vec![Sql::Integer(workspace_id)];
        if let Some(keyword) = keyword.filter(|k| !k.is_empty()) {
            filter.push_str(r#" AND ilike(u."name", ?)"#);
            binds.push(Sql::Text(like_contains(&keyword)));
        }
        let total: i64 = conn
            .query_row(
                &format!(
                    r#"SELECT COUNT(*) FROM "WorkspaceMember" m JOIN "User" u ON u."id" = m."userId" WHERE {filter}"#
                ),
                params_from_iter(binds.iter()),
                |row| row.get(0),
            )
            .map_err(repo_err)?;
        let mut page_binds = binds.clone();
        page_binds.push(Sql::Integer(pagination.limit));
        page_binds.push(Sql::Integer(pagination.offset));
        let mut statement = conn
            .prepare(&format!(
                r#"SELECT {COLUMNS} FROM "WorkspaceMember" m JOIN "User" u ON u."id" = m."userId" WHERE {filter} ORDER BY m."id" LIMIT ? OFFSET ?"#
            ))
            .map_err(repo_err)?;
        let data = statement
            .query_map(params_from_iter(page_binds.iter()), member)
            .map_err(repo_err)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(repo_err)?;
        let limit = pagination.limit.max(1);
        Ok(MemberPage {
            data,
            page_count: (total + limit - 1) / limit,
        })
    })
    .await
}

/// `findByIdWithUser`.
pub async fn find_by_id_with_user(
    db: &Db,
    workspace_id: i64,
    id: i64,
) -> Result<Option<MemberWithUser>> {
    tx::read(db, move |conn| {
        use chatbotx_db::rusqlite::OptionalExtension as _;
        conn.query_row(
            &format!(
                r#"SELECT {COLUMNS} FROM "WorkspaceMember" m JOIN "User" u ON u."id" = m."userId" WHERE m."id" = ?1 AND m."workspaceId" = ?2"#
            ),
            [id, workspace_id],
            member,
        )
        .optional()
        .map_err(repo_err)
    })
    .await
}

// ---- writes (Q.D11 slice 2) ----------------------------------------------

use chatbotx_db::repo::models::WorkspaceMember;
use chatbotx_db::repo::{find_one, now_ms};
use serde_json::Value;

use crate::BusinessError;
use crate::workspace_usage::{self, WorkspaceUsageMetric};

/// `FULL_WORKSPACE_MEMBER_PERMISSIONS` / `getSuperAdminPermissions`.
pub fn full_permissions() -> Value {
    crate::workspace::full_permissions()
}

/// `hasWorkspacePermission`: `superAdmin` grants every key; a missing key
/// fails closed.
pub fn has_permission(permissions: &Value, key: &str) -> bool {
    permissions.get("superAdmin") == Some(&Value::Bool(true))
        || permissions.get(key) == Some(&Value::Bool(true))
}

/// `hasContactsAccess`: `contacts` or `onlyAssignedContacts`.
pub fn has_contacts_access(permissions: &Value) -> bool {
    has_permission(permissions, "contacts") || has_permission(permissions, "onlyAssignedContacts")
}

/// `isWorkspaceAdminMember`: the owner, or a member whose stored
/// permissions parse with `superAdmin: true` (a malformed value is not).
pub fn is_workspace_admin_member(role: &str, permissions: &Value) -> bool {
    if role == "owner" {
        return true;
    }
    let Some(object) = permissions.as_object() else {
        return false;
    };
    // `workspaceMemberPermissionsSchema.partial()`: each known key that is
    // present is a boolean (unknown keys are stripped, not refused).
    crate::workspace::PERMISSION_KEYS
        .iter()
        .all(|key| object.get(*key).is_none_or(Value::is_boolean))
        && object.get("superAdmin") == Some(&Value::Bool(true))
}

/// `isSupportAccessEnabled`: `supportAccessUntil` set and in the future.
pub fn is_support_access_enabled(support_access_until: Option<i64>, now_ms: i64) -> bool {
    support_access_until.is_some_and(|until| until > now_ms)
}

/// `normalizeContactsPermissions`: full contacts access clears
/// `onlyAssignedContacts` (a new value; the input is not changed).
pub fn normalize_contacts_permissions(permissions: &Value) -> Value {
    let mut out = permissions.clone();
    if permissions.get("contacts") == Some(&Value::Bool(true))
        && let Some(object) = out.as_object_mut()
    {
        object.insert("onlyAssignedContacts".into(), Value::Bool(false));
    }
    out
}

/// `findByIdOrFail` (404 "Workspace member not found").
pub async fn find_by_id_or_fail(db: &Db, workspace_id: i64, id: i64) -> Result<WorkspaceMember> {
    tx::read(db, move |conn| {
        Ok(find_one::<WorkspaceMember>(
            conn,
            r#""id" = ?1 AND "workspaceId" = ?2"#,
            &[&id, &workspace_id],
        )?)
    })
    .await?
    .ok_or_else(|| BusinessError::not_found("Workspace member not found"))
}

/// The JSON fields of `update` (`updateWorkspaceMemberRequest`).
#[derive(Debug, Clone, PartialEq)]
pub struct MemberUpdate {
    pub permissions: Value,
    pub notification_types: Value,
    pub notification_channels: Value,
}

/// `update`: writes the member row of the workspace; `None` when no row
/// matched (a concurrent delete). Returns the member's user id; the caller
/// revokes realtime connections (`revokeRealtime`).
pub async fn update(
    db: &Db,
    workspace_id: i64,
    id: i64,
    data: MemberUpdate,
) -> Result<Option<i64>> {
    tx::write(db, move |tx| {
        use chatbotx_db::rusqlite::OptionalExtension as _;
        Ok(tx
            .query_row(
                r#"UPDATE "WorkspaceMember" SET "permissions" = ?3, "notificationTypes" = ?4,
                     "notificationChannels" = ?5, "updatedAt" = ?6
                   WHERE "id" = ?1 AND "workspaceId" = ?2 RETURNING "userId""#,
                chatbotx_db::rusqlite::params![
                    id,
                    workspace_id,
                    data.permissions.to_string(),
                    data.notification_types.to_string(),
                    data.notification_channels.to_string(),
                    now_ms()
                ],
                |row| row.get(0),
            )
            .optional()
            .map_err(chatbotx_db::repo::Error::from)?)
    })
    .await
}

/// A removed member: its user and the audit label (`name ?? email`).
#[derive(Debug, Clone, PartialEq)]
pub struct RemovedMember {
    pub user_id: i64,
    pub label: String,
}

/// `delete`: in one transaction removes the member row, the user's
/// working-hours schedule rows of the workspace and one `teamMembers` usage
/// unit. `None` when there was no such member (nothing else is written).
/// The caller audits `delete` "removed <label> from workspace" and revokes
/// the user's realtime connections (`deleted`).
pub async fn delete(db: &Db, workspace_id: i64, id: i64) -> Result<Option<RemovedMember>> {
    tx::write(db, move |tx| {
        use chatbotx_db::rusqlite::OptionalExtension as _;
        let member: Option<(i64, Option<String>, String)> = tx
            .query_row(
                r#"SELECT m."userId", u."name", u."email" FROM "WorkspaceMember" m
                   JOIN "User" u ON u."id" = m."userId" WHERE m."id" = ?1 AND m."workspaceId" = ?2"#,
                [id, workspace_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()
            .map_err(chatbotx_db::repo::Error::from)?;
        tx.execute(
            r#"DELETE FROM "WorkspaceMember" WHERE "id" = ?1 AND "workspaceId" = ?2"#,
            [id, workspace_id],
        )
        .map_err(chatbotx_db::repo::Error::from)?;
        let Some((user_id, name, email)) = member else {
            return Ok(None);
        };
        tx.execute(
            r#"DELETE FROM "WorkingHoursMember" WHERE "workspaceId" = ?1 AND "userId" = ?2"#,
            [workspace_id, user_id],
        )
        .map_err(chatbotx_db::repo::Error::from)?;
        workspace_usage::adjust(tx, workspace_id, WorkspaceUsageMetric::TeamMembers, -1)?;
        Ok(Some(RemovedMember {
            user_id,
            label: name.unwrap_or(email),
        }))
    })
    .await
}
