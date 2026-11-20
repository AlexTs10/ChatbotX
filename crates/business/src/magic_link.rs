//! `packages/business/src/magic-link/service.ts`.

use chatbotx_db::Db;
use chatbotx_db::repo::client::is_unique_violation_error;
use chatbotx_db::repo::models::MagicLink;
use chatbotx_db::repo::utils::{SortDir, order_by_sql, parse_order_by};
use chatbotx_db::repo::{self, SortItem, id_list, new_id, now_ms};
use chatbotx_db::rusqlite::{Connection, params};

use crate::errors::{BusinessError, Result};
use crate::tx;

const NOT_FOUND: &str = "Magic link not found";

fn taken(error: BusinessError) -> BusinessError {
    match error {
        BusinessError::Repo(repo) if is_unique_violation_error(&repo, None) => {
            BusinessError::validation("name", "Name is already taken")
        }
        other => other,
    }
}

fn find_in(conn: &Connection, workspace_id: i64, id: i64) -> Result<MagicLink> {
    repo::find_one::<MagicLink>(
        conn,
        r#""id" = ?1 AND "workspaceId" = ?2"#,
        &[&id, &workspace_id],
    )?
    .ok_or_else(|| BusinessError::not_found(NOT_FOUND))
}

/// `findOrFail`.
pub async fn find_or_fail(db: &Db, workspace_id: i64, id: i64) -> Result<MagicLink> {
    tx::read(db, move |c| find_in(c, workspace_id, id)).await
}

/// `findByName`.
pub async fn find_by_name(db: &Db, workspace_id: i64, name: String) -> Result<Option<MagicLink>> {
    tx::read(db, move |c| {
        Ok(repo::find_one::<MagicLink>(
            c,
            r#""workspaceId" = ?1 AND "name" = ?2"#,
            &[&workspace_id, &name],
        )?)
    })
    .await
}

/// `create` (a taken name is a `validation` error on `name`).
pub async fn create(db: &Db, workspace_id: i64, name: String, url: String) -> Result<MagicLink> {
    tx::write(db, move |c| {
        let id = new_id();
        let now = now_ms();
        c.execute(
            r#"INSERT INTO "MagicLink" ("id", "createdAt", "updatedAt", "workspaceId", "name", "url")
               VALUES (?1, ?2, ?2, ?3, ?4, ?5)"#,
            params![id, now, workspace_id, name, url],
        )
        .map_err(|e| BusinessError::Repo(e.into()))?;
        find_in(c, workspace_id, id)
    })
    .await
    .map_err(taken)
}

/// `update`: `None` keeps the stored value.
pub async fn update(
    db: &Db,
    workspace_id: i64,
    id: i64,
    name: Option<String>,
    url: Option<String>,
) -> Result<MagicLink> {
    tx::write(db, move |c| {
        let current = find_in(c, workspace_id, id)?;
        if name.is_none() && url.is_none() {
            // drizzle's `.set({})` throws (a 500 at the API); nothing changes.
            return Err(BusinessError::Repo(repo::Error::Message(
                "No values to set".into(),
            )));
        }
        c.execute(
            r#"UPDATE "MagicLink" SET "name" = ?3, "url" = ?4, "updatedAt" = ?5
               WHERE "id" = ?1 AND "workspaceId" = ?2"#,
            params![
                id,
                workspace_id,
                name.unwrap_or(current.name),
                url.unwrap_or(current.url),
                now_ms()
            ],
        )
        .map_err(|e| BusinessError::Repo(e.into()))?;
        find_in(c, workspace_id, id)
    })
    .await
    .map_err(taken)
}

/// `deleteMany` (no error for unknown ids).
pub async fn delete_many(db: &Db, workspace_id: i64, ids: Vec<i64>) -> Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    tx::write(db, move |c| {
        c.execute(
            r#"DELETE FROM "MagicLink" WHERE "workspaceId" = ?1 AND "id" IN (SELECT value FROM json_each(?2))"#,
            params![workspace_id, id_list(&ids)],
        )
        .map_err(|e| BusinessError::Repo(e.into()))?;
        Ok(())
    })
    .await
}

/// `delete`: 404 for an unknown id.
pub async fn delete(db: &Db, workspace_id: i64, id: i64) -> Result<()> {
    find_or_fail(db, workspace_id, id).await?;
    delete_many(db, workspace_id, vec![id]).await
}

/// `list`: `keyword` matches name or url (case-insensitive); `sort` as
/// `parseOrderByAsObject` (model columns only; none = by id).
pub async fn list(
    db: &Db,
    workspace_id: i64,
    keyword: Option<String>,
    sort: Option<Vec<SortItem>>,
    page: i64,
    per_page: i64,
) -> Result<(Vec<MagicLink>, i64)> {
    const COLUMNS: &[&str] = &["id", "createdAt", "updatedAt", "workspaceId", "name", "url"];
    let mut order = parse_order_by(COLUMNS, sort.as_deref(), None);
    if order.is_empty() {
        order.push(("id".to_owned(), SortDir::Asc));
    }
    let order = order_by_sql(&order, None);
    let per_page = per_page.max(1);
    let offset = (page.max(1) - 1) * per_page;
    let keyword = keyword.filter(|k| !k.is_empty());
    tx::read(db, move |c| {
        let filter = r#""workspaceId" = ?1 AND (?2 IS NULL OR instr(lower("name"), lower(?2)) > 0 OR instr(lower("url"), lower(?2)) > 0)"#;
        let rows: Vec<MagicLink> = repo::find_many(
            c,
            &format!(r#"WHERE {filter} {order} LIMIT ?3 OFFSET ?4"#),
            &[&workspace_id, &keyword, &per_page, &offset],
        )?;
        let total: i64 = c
            .query_row(
                &format!(r#"SELECT count(*) FROM "MagicLink" WHERE {filter}"#),
                params![workspace_id, keyword],
                |r| r.get(0),
            )
            .map_err(|e| BusinessError::Repo(e.into()))?;
        Ok((rows, (total + per_page - 1) / per_page))
    })
    .await
}
