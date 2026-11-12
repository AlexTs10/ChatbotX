//! Port of `repositories/whatsapp-business-account/repository.ts` (P3.3). Every write is
//! a compare-and-set on `revision` (an absent row has the virtual revision 0).
#![allow(
    unused_variables,
    unused_imports,
    dead_code,
    unreachable_code,
    clippy::all
)]

use rusqlite::Connection;

use super::models::WhatsappBusinessAccount;
use super::{Json, Result, json_text, new_id, now_ms};

/// `UpsertWhatsappBusinessAccountCredentialInput`.
#[derive(Debug, Clone, PartialEq)]
pub struct UpsertCredentialInput {
    pub workspace_id: i64,
    pub waba_id: String,
    pub business_id: String,
    pub credential: Json,
    pub granted_scopes: Vec<String>,
    pub scope_checked_at: i64,
    pub expected_revision: i64,
}

/// `findByWaba`.
pub fn find_by_waba(
    conn: &Connection,
    workspace_id: i64,
    waba_id: &str,
) -> Result<Option<WhatsappBusinessAccount>> {
    todo!("P3.3")
}

/// `upsertCredential`: inserts with revision 1, or replaces the credential when the row
/// still has `expected_revision`; `None` on a stale revision.
pub fn upsert_credential(
    conn: &Connection,
    input: &UpsertCredentialInput,
) -> Result<Option<WhatsappBusinessAccount>> {
    todo!("P3.3")
}

/// `updateScopeCache`: CAS on `expected_revision`.
pub fn update_scope_cache(
    conn: &Connection,
    workspace_id: i64,
    waba_id: &str,
    granted_scopes: &[String],
    scope_checked_at: i64,
    expected_revision: i64,
) -> Result<Option<WhatsappBusinessAccount>> {
    todo!("P3.3")
}

/// `markProvisioned`: CAS on `expected_revision`; keeps `creditLineId` when `None`.
pub fn mark_provisioned(
    conn: &Connection,
    workspace_id: i64,
    waba_id: &str,
    expected_revision: i64,
    credit_line_id: Option<&str>,
) -> Result<Option<WhatsappBusinessAccount>> {
    todo!("P3.3")
}

/// `deleteIfOrphaned`: deletes the record when no WhatsApp integration of the workspace
/// uses the WABA any more.
pub fn delete_if_orphaned(conn: &Connection, workspace_id: i64, waba_id: &str) -> Result<bool> {
    todo!("P3.3")
}
