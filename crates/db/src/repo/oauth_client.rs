//! Port of `repositories/oauth-client/repository.ts` (P3.3).

use rusqlite::Connection;

use super::Result;
use super::models::OAuthClient;

/// `findByClientId`.
pub fn find_by_client_id(conn: &Connection, client_id: &str) -> Result<Option<OAuthClient>> {
    super::find_one(conn, "\"clientId\" = ?1", &[&client_id])
}
