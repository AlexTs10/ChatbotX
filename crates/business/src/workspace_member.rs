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
