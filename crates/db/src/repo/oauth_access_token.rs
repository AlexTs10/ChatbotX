//! Port of `repositories/oauth-access-token/repository.ts` (P3.3).

use rusqlite::Connection;

use super::Result;

/// `deleteByUserAndClient`: returns the deleted count.
pub fn delete_by_user_and_client(
    conn: &Connection,
    user_id: i64,
    client_id: &str,
) -> Result<usize> {
    Ok(conn.execute(
        "DELETE FROM \"OAuthAccessToken\" WHERE \"userId\" = ?1 AND \"clientId\" = ?2",
        rusqlite::params![user_id, client_id],
    )?)
}

/// `deleteExpired`: deletes tokens with `expiresAt < now`; returns the deleted count.
pub fn delete_expired(conn: &Connection, now: i64) -> Result<usize> {
    Ok(conn.execute(
        "DELETE FROM \"OAuthAccessToken\" WHERE \"expiresAt\" < ?1",
        [now],
    )?)
}
