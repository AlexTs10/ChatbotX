//! `inboxService.list` / `findByIdOrFail` / `updateMarkReadOnOutbound`
//! (`packages/business/src/inbox/service.ts`) for the public `inboxes.*` routes.

use chatbotx_db::Db;
use chatbotx_db::repo::models::Inbox;
use chatbotx_db::repo::utils::{PaginationInput, get_pagination_with_defaults};
use chatbotx_db::repo::{Model, now_ms};
use chatbotx_db::rusqlite::{OptionalExtension, params};

use crate::{BusinessError, Result, tx};

fn not_found() -> BusinessError {
    BusinessError::not_found("Inbox not found")
}

/// `list`: the workspace's connected inboxes (`getPaginationWithDefaults`:
/// 20 per page unless `perPage`) and `pageCount = ceil(total / (perPage ?? 10))`,
/// as the original.
///
/// ponytail: `includes: ["integration"]` loads nothing; the public resource
/// drops the integration relations anyway.
pub async fn list(
    db: &Db,
    workspace_id: i64,
    page: Option<i64>,
    per_page: Option<i64>,
) -> Result<(Vec<Inbox>, i64)> {
    let paging = get_pagination_with_defaults(PaginationInput { page, per_page });
    tx::read(db, move |c| {
        let sql = format!(
            r#"SELECT {} FROM "Inbox" WHERE "workspaceId" = ?1 AND "status" = 'connected'
               ORDER BY "id" LIMIT ?2 OFFSET ?3"#,
            Inbox::columns_sql(None)
        );
        let rows = c
            .prepare(&sql)?
            .query_map(
                params![workspace_id, paging.limit, paging.offset],
                Inbox::from_row,
            )?
            .collect::<chatbotx_db::rusqlite::Result<Vec<_>>>()?;
        let total: i64 = c.query_row(
            r#"SELECT COUNT(*) FROM "Inbox" WHERE "workspaceId" = ?1 AND "status" = 'connected'"#,
            [workspace_id],
            |r| r.get(0),
        )?;
        let limit = per_page.unwrap_or(10);
        Ok((rows, (total + limit - 1).div_euclid(limit)))
    })
    .await
}

/// `findByIdOrFail` (workspace-scoped).
pub async fn find_by_id_or_fail(db: &Db, workspace_id: i64, id: i64) -> Result<Inbox> {
    tx::read(db, move |c| {
        let sql = format!(
            r#"SELECT {} FROM "Inbox" WHERE "id" = ?1 AND "workspaceId" = ?2"#,
            Inbox::columns_sql(None)
        );
        Ok(
            c.query_row(&sql, params![id, workspace_id], Inbox::from_row)
                .optional()?,
        )
    })
    .await?
    .ok_or_else(not_found)
}

/// `updateMarkReadOnOutbound` (workspace-scoped; 404 when no row).
pub async fn update_mark_read_on_outbound(
    db: &Db,
    workspace_id: i64,
    id: i64,
    enabled: bool,
) -> Result<Inbox> {
    tx::write(db, move |c| {
        let sql = format!(
            r#"UPDATE "Inbox" SET "markReadOnOutbound" = ?1, "updatedAt" = ?2
               WHERE "id" = ?3 AND "workspaceId" = ?4 RETURNING {}"#,
            Inbox::columns_sql(None)
        );
        Ok(c.query_row(
            &sql,
            params![enabled, now_ms(), id, workspace_id],
            Inbox::from_row,
        )
        .optional()?)
    })
    .await?
    .ok_or_else(not_found)
}
