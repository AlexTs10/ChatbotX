//! Port of `repositories/whatsapp-business-account/repository.ts` (P3.3). Every write is
//! a compare-and-set on `revision` (an absent row has the virtual revision 0).

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
    super::find_one(
        conn,
        "\"workspaceId\" = ?1 AND \"wabaId\" = ?2",
        &[&workspace_id, &waba_id],
    )
}

/// `upsertCredential`: inserts with revision 1, or replaces the credential when the row
/// still has `expected_revision`; `None` on a stale revision.
pub fn upsert_credential(
    conn: &Connection,
    input: &UpsertCredentialInput,
) -> Result<Option<WhatsappBusinessAccount>> {
    let now = now_ms();
    let scopes = json_text(&input.granted_scopes)?;
    let rows = super::returning::<WhatsappBusinessAccount>(
        conn,
        "INSERT INTO \"WhatsappBusinessAccount\" (\"id\", \"createdAt\", \"updatedAt\", \"workspaceId\", \"wabaId\", \
         \"businessId\", \"credential\", \"grantedScopes\", \"scopeCheckedAt\", \"revision\") \
         VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 1) \
         ON CONFLICT (\"workspaceId\", \"wabaId\") DO UPDATE SET \"businessId\" = excluded.\"businessId\", \
         \"credential\" = excluded.\"credential\", \"grantedScopes\" = excluded.\"grantedScopes\", \
         \"scopeCheckedAt\" = excluded.\"scopeCheckedAt\", \"revision\" = \"WhatsappBusinessAccount\".\"revision\" + 1, \
         \"updatedAt\" = excluded.\"updatedAt\" \
         WHERE \"WhatsappBusinessAccount\".\"revision\" = ?9",
        &[
            &new_id(),
            &now,
            &input.workspace_id,
            &input.waba_id,
            &input.business_id,
            &input.credential,
            &scopes,
            &input.scope_checked_at,
            &input.expected_revision,
        ],
    )?;
    Ok(rows.into_iter().next())
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
    let rows = super::returning::<WhatsappBusinessAccount>(
        conn,
        "UPDATE \"WhatsappBusinessAccount\" SET \"grantedScopes\" = ?3, \"scopeCheckedAt\" = ?4, \
         \"revision\" = \"revision\" + 1, \"updatedAt\" = ?6 \
         WHERE \"workspaceId\" = ?1 AND \"wabaId\" = ?2 AND \"revision\" = ?5",
        &[
            &workspace_id,
            &waba_id,
            &json_text(granted_scopes)?,
            &scope_checked_at,
            &expected_revision,
            &now_ms(),
        ],
    )?;
    Ok(rows.into_iter().next())
}

/// `markProvisioned`: CAS on `expected_revision`; keeps `creditLineId` when `None`.
pub fn mark_provisioned(
    conn: &Connection,
    workspace_id: i64,
    waba_id: &str,
    expected_revision: i64,
    credit_line_id: Option<&str>,
) -> Result<Option<WhatsappBusinessAccount>> {
    let now = now_ms();
    let rows = super::returning::<WhatsappBusinessAccount>(
        conn,
        "UPDATE \"WhatsappBusinessAccount\" SET \"provisionedAt\" = ?4, \
         \"creditLineId\" = coalesce(?5, \"creditLineId\"), \"revision\" = \"revision\" + 1, \"updatedAt\" = ?4 \
         WHERE \"workspaceId\" = ?1 AND \"wabaId\" = ?2 AND \"revision\" = ?3",
        &[
            &workspace_id,
            &waba_id,
            &expected_revision,
            &now,
            &credit_line_id,
        ],
    )?;
    Ok(rows.into_iter().next())
}

/// `deleteIfOrphaned`: deletes the record when no WhatsApp integration of the workspace
/// uses the WABA any more.
pub fn delete_if_orphaned(conn: &Connection, workspace_id: i64, waba_id: &str) -> Result<bool> {
    let deleted = conn.execute(
        "DELETE FROM \"WhatsappBusinessAccount\" WHERE \"workspaceId\" = ?1 AND \"wabaId\" = ?2 \
         AND NOT EXISTS (SELECT 1 FROM \"IntegrationWhatsapp\" i WHERE i.\"workspaceId\" = ?1 AND i.\"wabaId\" = ?2)",
        rusqlite::params![workspace_id, waba_id],
    )?;
    Ok(deleted > 0)
}
