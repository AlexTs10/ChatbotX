//! Port of `repositories/license/repository.ts` (P3.3).

use rusqlite::Connection;

use super::Result;

/// `findTokenByLicenseId`.
pub fn find_token_by_license_id(conn: &Connection, license_id: &str) -> Result<Option<String>> {
    let _ = (conn, license_id);
    todo!("P3.3")
}

/// `upsert`: inserts the license, or replaces token and expiry of the existing one.
pub fn upsert(conn: &Connection, license_id: &str, token: &str, expires_at: i64) -> Result<()> {
    let _ = (conn, license_id, token, expires_at);
    todo!("P3.3")
}
