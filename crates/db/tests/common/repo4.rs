//! Shared fixtures of the P3.3 (rest) and P3.2b repository ports (on top of `common/repo.rs`).
//!
//! Include after `repo_common`:
//! ```ignore
//! #[path = "common/repo4.rs"]
//! mod repo4;
//! ```

#![allow(dead_code)]

use chatbotx_db::DbError;
use chatbotx_db::rusqlite::{Connection, params};

/// Inserts one `User` (`email` = `user-<id>@example.test`).
pub fn user(tx: &Connection, id: i64) -> Result<i64, DbError> {
    tx.execute(
        r#"INSERT INTO "User" ("id", "email") VALUES (?1, ?2)"#,
        params![id, format!("user-{id}@example.test")],
    )?;
    Ok(id)
}

/// Inserts one `User` and a `Workspace` that it owns.
pub fn workspace(tx: &Connection, user_id: i64, workspace_id: i64) -> Result<i64, DbError> {
    user(tx, user_id)?;
    tx.execute(
        r#"INSERT INTO "Workspace" ("id", "name", "ownerId") VALUES (?1, ?2, ?3)"#,
        params![workspace_id, format!("Workspace {workspace_id}"), user_id],
    )?;
    Ok(workspace_id)
}

/// Inserts one `WorkspaceMember` (`role` = `agent`); `id` = `user_id * 10 + 7`.
pub fn member(tx: &Connection, workspace_id: i64, user_id: i64) -> Result<(), DbError> {
    tx.execute(
        r#"INSERT INTO "WorkspaceMember" ("id", "workspaceId", "userId", "role")
           VALUES (?1, ?2, ?3, 'agent')"#,
        params![user_id * 10 + 7, workspace_id, user_id],
    )?;
    Ok(())
}

/// Inserts one `Product` (`status` = `status`, `sku` optional).
pub fn product(
    tx: &Connection,
    id: i64,
    workspace_id: i64,
    name: &str,
    status: &str,
    sku: Option<&str>,
) -> Result<i64, DbError> {
    tx.execute(
        r#"INSERT INTO "Product" ("id", "name", "workspaceId", "status", "sku")
           VALUES (?1, ?2, ?3, ?4, ?5)"#,
        params![id, name, workspace_id, status, sku],
    )?;
    Ok(id)
}

/// Inserts one `Integration` (`metaCatalog`) and one `IntegrationMetaCatalog` on it.
/// Returns the connection id.
pub fn meta_catalog(
    tx: &Connection,
    id: i64,
    workspace_id: i64,
    catalog_id: &str,
    deleted: bool,
) -> Result<i64, DbError> {
    tx.execute(
        r#"INSERT INTO "Integration" ("id", "workspaceId", "integrationType") VALUES (?1, ?2, 'metaCatalog')"#,
        params![id + 900_000, workspace_id],
    )?;
    tx.execute(
        r#"INSERT INTO "IntegrationMetaCatalog" ("id", "workspaceId", "integrationId", "catalogId", "deletedAt")
           VALUES (?1, ?2, ?3, ?4, ?5)"#,
        params![
            id,
            workspace_id,
            id + 900_000,
            catalog_id,
            deleted.then_some(1_i64)
        ],
    )?;
    Ok(id)
}

/// Inserts one `MetaCatalogItem` (`id` = `product_id * 10 + connection id % 10`).
pub fn catalog_item(
    tx: &Connection,
    connection_id: i64,
    catalog_id: &str,
    product_id: i64,
    retailer_id: &str,
) -> Result<(), DbError> {
    tx.execute(
        r#"INSERT INTO "MetaCatalogItem" ("id", "integrationMetaCatalogId", "catalogId", "productId", "retailerId")
           VALUES (?1, ?2, ?3, ?4, ?5)"#,
        params![
            product_id * 10 + connection_id % 10,
            connection_id,
            catalog_id,
            product_id,
            retailer_id
        ],
    )?;
    Ok(())
}

/// Inserts one `WorkingHours` row with `updatedAt` = `updated_at`.
pub fn working_hours(
    tx: &Connection,
    id: i64,
    workspace_id: i64,
    name: &str,
    updated_at: i64,
) -> Result<i64, DbError> {
    tx.execute(
        r#"INSERT INTO "WorkingHours" ("id", "workspaceId", "name", "updatedAt") VALUES (?1, ?2, ?3, ?4)"#,
        params![id, workspace_id, name, updated_at],
    )?;
    Ok(id)
}

/// Inserts one `WorkingHoursMember` (`id` = `user_id * 100 + working_hours_id % 100`).
pub fn working_hours_member(
    tx: &Connection,
    working_hours_id: i64,
    workspace_id: i64,
    user_id: i64,
) -> Result<(), DbError> {
    tx.execute(
        r#"INSERT INTO "WorkingHoursMember" ("id", "workingHoursId", "workspaceId", "userId")
           VALUES (?1, ?2, ?3, ?4)"#,
        params![
            user_id * 100 + working_hours_id % 100,
            working_hours_id,
            workspace_id,
            user_id
        ],
    )?;
    Ok(())
}

/// `new Date(Date.UTC(2030, 0, 1 + offset)).toISOString().slice(0, 10)`.
pub fn date_of(offset: i64) -> String {
    let base = 1_893_456_000_000_i64; // 2030-01-01T00:00:00Z
    chatbotx_db::repo::iso(base + offset * 86_400_000)[..10].to_owned()
}
