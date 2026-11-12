//! Port of `repositories/platform-admin/repository.ts` (P3.3).

use rusqlite::Connection;

use super::models::PlatformAdmin;
use super::utils::{PaginationInput, get_pagination_with_defaults};
use super::{Result, new_id, now_ms, user};

/// `ROOT_TENANT_ID` (`partials/shared.ts`).
pub const ROOT_TENANT_ID: i64 = 1;

/// `PlatformAdminWithUser`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlatformAdminWithUser {
    pub id: i64,
    pub created_at: i64,
    pub updated_at: i64,
    pub user_id: i64,
    pub role: String,
    pub user_email: String,
    pub user_name: Option<String>,
}

/// Result of [`list_with_user`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlatformAdminPage {
    pub data: Vec<PlatformAdminWithUser>,
    pub page_count: i64,
}

/// `findForAuth`: always a fresh read (no cache).
pub fn find_for_auth(conn: &Connection, user_id: i64) -> Result<Option<PlatformAdmin>> {
    super::find_one(conn, "\"userId\" = ?1", &[&user_id])
}

/// `bootstrapRootAdmin`: makes the root-tenant user with `email` a `super-admin`.
/// `true` when a row was created or promoted; `false` when it already was super-admin or
/// no such user exists.
pub fn bootstrap_root_admin(conn: &Connection, email: &str) -> Result<bool> {
    let Some(user_id) = user::find_id_by_email(conn, email, ROOT_TENANT_ID)? else {
        return Ok(false);
    };
    let now = now_ms();
    let changed = conn.execute(
        "INSERT INTO \"PlatformAdmin\" (\"id\", \"createdAt\", \"updatedAt\", \"userId\", \"role\") \
         VALUES (?1, ?2, ?2, ?3, 'super-admin') \
         ON CONFLICT (\"userId\") DO UPDATE SET \"role\" = 'super-admin', \"updatedAt\" = excluded.\"updatedAt\" \
         WHERE \"PlatformAdmin\".\"role\" <> 'super-admin'",
        rusqlite::params![new_id(), now, user_id],
    )?;
    Ok(changed > 0)
}

/// `listWithUser`: newest first, with the user's email and name.
pub fn list_with_user(conn: &Connection, input: PaginationInput) -> Result<PlatformAdminPage> {
    let pagination = get_pagination_with_defaults(input);
    let mut stmt = conn.prepare_cached(
        "SELECT pa.\"id\", pa.\"createdAt\", pa.\"updatedAt\", pa.\"userId\", pa.\"role\", u.\"email\", u.\"name\" \
         FROM \"PlatformAdmin\" pa INNER JOIN \"User\" u ON pa.\"userId\" = u.\"id\" \
         ORDER BY pa.\"createdAt\" DESC LIMIT ?1 OFFSET ?2",
    )?;
    let data = stmt
        .query_map(
            rusqlite::params![pagination.limit, pagination.offset],
            |row| {
                Ok(PlatformAdminWithUser {
                    id: row.get(0)?,
                    created_at: row.get(1)?,
                    updated_at: row.get(2)?,
                    user_id: row.get(3)?,
                    role: row.get(4)?,
                    user_email: row.get(5)?,
                    user_name: row.get(6)?,
                })
            },
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let count: i64 = conn.query_row("SELECT count(*) FROM \"PlatformAdmin\"", [], |r| r.get(0))?;
    // `Math.ceil(count / limit)` (a zero limit gives the JS `Infinity`/`NaN`; clamp to 0).
    let page_count = if pagination.limit > 0 {
        (count + pagination.limit - 1) / pagination.limit
    } else {
        0
    };
    Ok(PlatformAdminPage { data, page_count })
}

/// `upsertRole`: inserts the admin row or sets its role.
pub fn upsert_role(conn: &Connection, role: &str, user_id: i64) -> Result<Option<PlatformAdmin>> {
    let now = now_ms();
    let rows = super::returning::<PlatformAdmin>(
        conn,
        "INSERT INTO \"PlatformAdmin\" (\"id\", \"createdAt\", \"updatedAt\", \"userId\", \"role\") \
         VALUES (?1, ?2, ?2, ?3, ?4) \
         ON CONFLICT (\"userId\") DO UPDATE SET \"role\" = excluded.\"role\", \"updatedAt\" = excluded.\"updatedAt\"",
        &[&new_id(), &now, &user_id, &role],
    )?;
    Ok(rows.into_iter().next())
}

/// `deleteByUserId`.
pub fn delete_by_user_id(conn: &Connection, user_id: i64) -> Result<()> {
    conn.execute(
        "DELETE FROM \"PlatformAdmin\" WHERE \"userId\" = ?1",
        [user_id],
    )?;
    Ok(())
}
