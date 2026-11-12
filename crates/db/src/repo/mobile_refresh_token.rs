//! Port of `repositories/mobile-refresh-token/repository.ts` (P3.3).
//!
//! `pg_advisory_xact_lock` is not needed: the one SQLite writer serializes every write
//! job, so [`lock_family_by_hash`] only re-reads the row (spec P3.2 section 2).

use rusqlite::Connection;

use super::Result;
use super::models::MobileRefreshToken;

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
    let _ = (conn, values);
    todo!("P3.3")
}

/// `spendActive`: marks a live token (not rotated, not revoked, not expired) as rotated
/// at `now` and returns it; `None` when no live token matches.
pub fn spend_active(
    conn: &Connection,
    token_hash: &str,
    now: i64,
) -> Result<Option<MobileRefreshToken>> {
    let _ = (conn, token_hash, now);
    todo!("P3.3")
}

/// `findByHash`.
pub fn find_by_hash(conn: &Connection, token_hash: &str) -> Result<Option<MobileRefreshToken>> {
    let _ = (conn, token_hash);
    todo!("P3.3")
}

/// `lockFamilyByHash`: the row of `token_hash`, read inside the caller's write job.
pub fn lock_family_by_hash(
    conn: &Connection,
    token_hash: &str,
) -> Result<Option<MobileRefreshToken>> {
    let _ = (conn, token_hash);
    todo!("P3.3")
}

/// `revokeFamilyByHash`: revokes every not-yet-revoked token of the family of `token_hash`.
pub fn revoke_family_by_hash(conn: &Connection, token_hash: &str, now: i64) -> Result<()> {
    let _ = (conn, token_hash, now);
    todo!("P3.3")
}

/// `revokeAllForUser`.
pub fn revoke_all_for_user(conn: &Connection, user_id: i64, now: i64) -> Result<()> {
    let _ = (conn, user_id, now);
    todo!("P3.3")
}

/// `deleteSpentInFamily`: deletes tokens of the family rotated before `before`.
pub fn delete_spent_in_family(conn: &Connection, family_id: &str, before: i64) -> Result<()> {
    let _ = (conn, family_id, before);
    todo!("P3.3")
}

/// `deleteExpiredForUser`.
pub fn delete_expired_for_user(conn: &Connection, user_id: i64, now: i64) -> Result<()> {
    let _ = (conn, user_id, now);
    todo!("P3.3")
}

/// `deleteExpired`: deletes expired tokens and tokens revoked more than 24 h ago;
/// returns the deleted count.
pub fn delete_expired(conn: &Connection, now: i64) -> Result<usize> {
    let _ = (conn, now);
    todo!("P3.3")
}
