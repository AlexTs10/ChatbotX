//! Port of `repositories/mobile-refresh-token/repository.ts` (P3.3).
//!
//! `pg_advisory_xact_lock` is not needed: the one SQLite writer serializes every write
//! job, so [`lock_family_by_hash`] only re-reads the row (spec P3.2 section 2).

use rusqlite::Connection;

use super::models::MobileRefreshToken;
use super::{Result, new_id, now_ms};

/// `REVOKED_TOKEN_RETENTION_MS` (24 h).
pub const REVOKED_TOKEN_RETENTION_MS: i64 = 24 * 60 * 60 * 1000;

/// `MobileRefreshTokenInsert`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewMobileRefreshToken {
    pub user_id: i64,
    pub device_id: String,
    pub family_id: String,
    pub token_hash: String,
    pub expires_at: i64,
    pub rotated_at: Option<i64>,
    pub revoked: Option<i64>,
}

/// `insert`.
pub fn insert(conn: &Connection, values: &NewMobileRefreshToken) -> Result<()> {
    let now = now_ms();
    conn.execute(
        "INSERT INTO \"MobileRefreshToken\" (\"id\", \"createdAt\", \"updatedAt\", \"userId\", \"deviceId\", \
         \"familyId\", \"tokenHash\", \"expiresAt\", \"rotatedAt\", \"revoked\") \
         VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        rusqlite::params![
            new_id(),
            now,
            values.user_id,
            values.device_id,
            values.family_id,
            values.token_hash,
            values.expires_at,
            values.rotated_at,
            values.revoked
        ],
    )?;
    Ok(())
}

/// `spendActive`: marks a live token (not rotated, not revoked, not expired) as rotated
/// at `now` and returns it; `None` when no live token matches.
pub fn spend_active(
    conn: &Connection,
    token_hash: &str,
    now: i64,
) -> Result<Option<MobileRefreshToken>> {
    let rows = super::returning::<MobileRefreshToken>(
        conn,
        "UPDATE \"MobileRefreshToken\" SET \"rotatedAt\" = ?2, \"updatedAt\" = ?3 \
         WHERE \"tokenHash\" = ?1 AND \"rotatedAt\" IS NULL AND \"revoked\" IS NULL AND \"expiresAt\" > ?2",
        &[&token_hash, &now, &now_ms()],
    )?;
    Ok(rows.into_iter().next())
}

/// `findByHash`.
pub fn find_by_hash(conn: &Connection, token_hash: &str) -> Result<Option<MobileRefreshToken>> {
    super::find_one(conn, "\"tokenHash\" = ?1", &[&token_hash])
}

/// `lockFamilyByHash`: the row of `token_hash`, read inside the caller's write job.
pub fn lock_family_by_hash(
    conn: &Connection,
    token_hash: &str,
) -> Result<Option<MobileRefreshToken>> {
    find_by_hash(conn, token_hash)
}

/// `revokeFamilyByHash`: revokes every not-yet-revoked token of the family of `token_hash`.
pub fn revoke_family_by_hash(conn: &Connection, token_hash: &str, now: i64) -> Result<()> {
    conn.execute(
        "UPDATE \"MobileRefreshToken\" SET \"revoked\" = ?2, \"updatedAt\" = ?3 \
         WHERE \"familyId\" IN (SELECT \"familyId\" FROM \"MobileRefreshToken\" WHERE \"tokenHash\" = ?1) \
         AND \"revoked\" IS NULL",
        rusqlite::params![token_hash, now, now_ms()],
    )?;
    Ok(())
}

/// `revokeAllForUser`.
pub fn revoke_all_for_user(conn: &Connection, user_id: i64, now: i64) -> Result<()> {
    conn.execute(
        "UPDATE \"MobileRefreshToken\" SET \"revoked\" = ?2, \"updatedAt\" = ?3 \
         WHERE \"userId\" = ?1 AND \"revoked\" IS NULL",
        rusqlite::params![user_id, now, now_ms()],
    )?;
    Ok(())
}

/// `deleteSpentInFamily`: deletes tokens of the family rotated before `before`.
pub fn delete_spent_in_family(conn: &Connection, family_id: &str, before: i64) -> Result<()> {
    conn.execute(
        "DELETE FROM \"MobileRefreshToken\" WHERE \"familyId\" = ?1 AND \"rotatedAt\" < ?2",
        rusqlite::params![family_id, before],
    )?;
    Ok(())
}

/// `deleteExpiredForUser`.
pub fn delete_expired_for_user(conn: &Connection, user_id: i64, now: i64) -> Result<()> {
    conn.execute(
        "DELETE FROM \"MobileRefreshToken\" WHERE \"userId\" = ?1 AND \"expiresAt\" < ?2",
        rusqlite::params![user_id, now],
    )?;
    Ok(())
}

/// `deleteExpired`: deletes expired tokens and tokens revoked more than 24 h ago;
/// returns the deleted count.
pub fn delete_expired(conn: &Connection, now: i64) -> Result<usize> {
    Ok(conn.execute(
        "DELETE FROM \"MobileRefreshToken\" WHERE \"expiresAt\" < ?1 OR \"revoked\" < ?2",
        rusqlite::params![now, now - REVOKED_TOKEN_RETENTION_MS],
    )?)
}
