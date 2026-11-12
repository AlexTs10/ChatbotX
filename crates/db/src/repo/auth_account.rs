//! Port of `repositories/auth-account/repository.ts` (P3.3).

use rusqlite::{Connection, OptionalExtension};

use super::Result;

/// `AuthAccountToken`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthAccountToken {
    pub access_token: Option<String>,
}

/// `findByUserAndProvider`: a user's linked account of one provider (`userId` is
/// tenant-safe on its own).
pub fn find_by_user_and_provider(
    conn: &Connection,
    user_id: i64,
    provider_id: &str,
) -> Result<Option<AuthAccountToken>> {
    let mut stmt = conn.prepare_cached(
        "SELECT \"accessToken\" FROM \"Account\" WHERE \"userId\" = ?1 AND \"providerId\" = ?2 LIMIT 1",
    )?;
    Ok(stmt
        .query_row(rusqlite::params![user_id, provider_id], |row| {
            Ok(AuthAccountToken {
                access_token: row.get(0)?,
            })
        })
        .optional()?)
}
