//! Port of `repositories/platform-admin/repository.ts` (P3.3).

use rusqlite::Connection;

use super::Result;
use super::models::PlatformAdmin;
use super::utils::PaginationInput;

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
    let _ = (conn, user_id);
    todo!("P3.3")
}

/// `bootstrapRootAdmin`: makes the root-tenant user with `email` a `super-admin`.
/// `true` when a row was created or promoted; `false` when it already was super-admin or
/// no such user exists.
pub fn bootstrap_root_admin(conn: &Connection, email: &str) -> Result<bool> {
    let _ = (conn, email);
    todo!("P3.3")
}

/// `listWithUser`: newest first, with the user's email and name.
pub fn list_with_user(conn: &Connection, input: PaginationInput) -> Result<PlatformAdminPage> {
    let _ = (conn, input);
    todo!("P3.3")
}

/// `upsertRole`: inserts the admin row or sets its role.
pub fn upsert_role(conn: &Connection, role: &str, user_id: i64) -> Result<Option<PlatformAdmin>> {
    let _ = (conn, role, user_id);
    todo!("P3.3")
}

/// `deleteByUserId`.
pub fn delete_by_user_id(conn: &Connection, user_id: i64) -> Result<()> {
    let _ = (conn, user_id);
    todo!("P3.3")
}
