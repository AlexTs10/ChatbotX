//! Port of `repositories/license/repository.ts` (P3.3).

use rusqlite::{Connection, OptionalExtension};

use super::{Result, new_id, now_ms};

/// `findTokenByLicenseId`.
pub fn find_token_by_license_id(conn: &Connection, license_id: &str) -> Result<Option<String>> {
    let mut stmt =
        conn.prepare_cached("SELECT \"token\" FROM \"License\" WHERE \"licenseId\" = ?1 LIMIT 1")?;
    Ok(stmt.query_row([license_id], |row| row.get(0)).optional()?)
}

/// `upsert`: inserts the license, or replaces token and expiry of the existing one.
pub fn upsert(conn: &Connection, license_id: &str, token: &str, expires_at: i64) -> Result<()> {
    let now = now_ms();
    conn.execute(
        "INSERT INTO \"License\" (\"id\", \"createdAt\", \"updatedAt\", \"licenseId\", \"token\", \"expiresAt\") \
         VALUES (?1, ?2, ?2, ?3, ?4, ?5) \
         ON CONFLICT (\"licenseId\") DO UPDATE SET \"token\" = excluded.\"token\", \
         \"expiresAt\" = excluded.\"expiresAt\", \"updatedAt\" = excluded.\"updatedAt\"",
        rusqlite::params![new_id(), now, license_id, token, expires_at],
    )?;
    Ok(())
}
