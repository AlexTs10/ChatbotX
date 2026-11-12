//! Port of `repositories/whatsapp-call/permission-repository.ts` (P3.3; no TS tests, covered by
//! `tests/repo_p33_smoke.rs`). Every UPDATE sets `"updatedAt"` (Drizzle `$onUpdate`).

use rusqlite::Connection;
use rusqlite::types::Value;

use super::models::WhatsappCallPermission;
use super::sql::{Sql, bool_val, opt_int};
use super::{Model, Result, new_id, now_ms};

/// `UpsertPermissionInput`.
#[derive(Debug, Clone, PartialEq)]
pub struct UpsertPermissionInput {
    pub workspace_id: i64,
    pub contact_inbox_id: i64,
    pub response: String,
    pub is_permanent: bool,
    pub expires_at: Option<i64>,
    pub responded_at: i64,
}

/// `findByContactInboxId`.
pub fn find_by_contact_inbox_id(
    conn: &Connection,
    contact_inbox_id: i64,
) -> Result<Option<WhatsappCallPermission>> {
    todo!("P3.3")
}

/// `upsertForContactInbox`: the newer response wins (`DO UPDATE ... WHERE respondedAt <=
/// excluded`); `None` when the stored response is newer.
pub fn upsert_for_contact_inbox(
    conn: &Connection,
    input: &UpsertPermissionInput,
) -> Result<Option<WhatsappCallPermission>> {
    todo!("P3.3")
}
