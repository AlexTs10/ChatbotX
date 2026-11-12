//! Port of `repositories/user/repository.ts` (P3.3).

use rusqlite::{Connection, OptionalExtension};

use super::client::is_unique_violation_error;
use super::{Error, Result, new_id, now_ms};
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
    let mut stmt = conn.prepare_cached(
        "SELECT \"id\" FROM \"User\" WHERE \"email\" = ?1 AND \"tenantId\" = ?2 LIMIT 1",
    )?;
    Ok(stmt
        .query_row(rusqlite::params![email, tenant_id], |row| row.get(0))
        .optional()?)
}

/// `insertWithCredential`: one transaction that creates a verified user with a forced
/// password change and its `credential` account. On a unique violation (a concurrent
/// insert of the same tenant-scoped email) it returns the winner with `created: false`.
pub async fn insert_with_credential(
    db: &Db,
    input: InsertWithCredentialInput,
) -> Result<InsertWithCredentialResult> {
    let values = input.clone();
    let inserted = super::write(db, move |tx| {
        let now = now_ms();
        let user_id = new_id();
        tx.execute(
            "INSERT INTO \"User\" (\"id\", \"createdAt\", \"updatedAt\", \"email\", \"name\", \
             \"emailVerified\", \"mustChangePassword\", \"tenantId\") \
             VALUES (?1, ?2, ?2, ?3, ?4, 1, 1, ?5)",
            rusqlite::params![user_id, now, values.email, values.name, values.tenant_id],
        )?;
        tx.execute(
            "INSERT INTO \"Account\" (\"id\", \"createdAt\", \"updatedAt\", \"accountId\", \
             \"providerId\", \"password\", \"userId\", \"tenantId\") \
             VALUES (?1, ?2, ?2, ?3, 'credential', ?4, ?5, ?6)",
            rusqlite::params![
                new_id(),
                now,
                user_id.to_string(),
                values.hashed_password,
                user_id,
                values.tenant_id
            ],
        )?;
        Ok(user_id)
    })
    .await;
    match inserted {
        Ok(user_id) => Ok(InsertWithCredentialResult {
            user_id,
            created: true,
        }),
        Err(error) => {
            if is_unique_violation_error(&error, None) {
                let email = input.email.clone();
                let tenant_id = input.tenant_id;
                let raced =
                    super::read(db, move |c| find_id_by_email(c, &email, tenant_id)).await?;
                if let Some(user_id) = raced {
                    return Ok(InsertWithCredentialResult {
                        user_id,
                        created: false,
                    });
                }
            }
            Err::<InsertWithCredentialResult, Error>(error)
        }
    }
}
