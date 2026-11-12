//! Port of `repositories/user/repository.ts` (P3.3).

use rusqlite::Connection;

use super::Result;
use crate::Db;

/// Input of [`insert_with_credential`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InsertWithCredentialInput {
    pub email: String,
    pub name: Option<String>,
    pub hashed_password: String,
    pub tenant_id: i64,
}

/// Result of [`insert_with_credential`] (`{ userId, created }`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InsertWithCredentialResult {
    pub user_id: i64,
    pub created: bool,
}

/// `findIdByEmail`: the id of the user with `email` inside `tenant_id`.
pub fn find_id_by_email(conn: &Connection, email: &str, tenant_id: i64) -> Result<Option<i64>> {
    let _ = (conn, email, tenant_id);
    todo!("P3.3")
}

/// `insertWithCredential`: one transaction that creates a verified user with a forced
/// password change and its `credential` account. On a unique violation (a concurrent
/// insert of the same tenant-scoped email) it returns the winner with `created: false`.
pub async fn insert_with_credential(
    db: &Db,
    input: InsertWithCredentialInput,
) -> Result<InsertWithCredentialResult> {
    let _ = (db, input);
    todo!("P3.3")
}
