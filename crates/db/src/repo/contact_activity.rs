//! Port of `repositories/contact-activity/repository.ts`. `FOR KEY SHARE` row locks are
//! not needed (one writer); the join on `Contact` keeps the workspace scope.

use rusqlite::Connection;

use super::{Result, id_list};
use crate::Db;

/// `ContactActivityRecord`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContactActivityRecord {
    pub activity_type: String,
    pub contact_id: i64,
    pub occurred_at: i64,
    pub parent_reference_id: i64,
    pub reference_id: i64,
    pub reference_type: String,
}

/// `ContactKeywordMessageRecord`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContactKeywordMessageRecord {
    pub automated_response_id: i64,
    pub contact_id: i64,
    pub message_id: i64,
    pub normalized_text: String,
    pub occurred_at: i64,
    pub text: String,
}

/// `CONTACT_ACTIVITY_TABLE_KEYS`: each table's key columns after `workspaceId`.
pub const CONTACT_ACTIVITY_TABLE_KEYS: &[(&str, &[&str])] = &[
    ("ContactActivity", &["contactId", "activityType", "referenceType", "parentReferenceId", "referenceId"]),
    ("ContactKeywordMessage", &["contactId", "messageId"]),
    ("ContactState", &["contactId"]),
];

fn keys(table: &str) -> Option<&'static [&'static str]> {
    CONTACT_ACTIVITY_TABLE_KEYS.iter().find(|(t, _)| *t == table).map(|(_, k)| *k)
}

/// `upsertActivities`: inserts the records of the workspace's contacts (others are
/// skipped); an existing row only moves `firstOccurredAt` earlier. Returns the number of
/// written rows.
pub fn upsert_activities(conn: &Connection, workspace_id: i64, records: &[ContactActivityRecord]) -> Result<usize> {
    todo!("P3.2");
    let mut stmt = conn.prepare_cached(
        "INSERT INTO \"ContactActivity\" (\"workspaceId\", \"contactId\", \"activityType\", \"referenceType\", \"parentReferenceId\", \"referenceId\", \"firstOccurredAt\") \
         SELECT c.\"workspaceId\", c.\"id\", ?2, ?3, ?4, ?5, ?6 FROM \"Contact\" AS c WHERE c.\"id\" = ?1 AND c.\"workspaceId\" = ?7 \
         ON CONFLICT (\"workspaceId\", \"contactId\", \"activityType\", \"referenceType\", \"parentReferenceId\", \"referenceId\") \
         DO UPDATE SET \"firstOccurredAt\" = excluded.\"firstOccurredAt\" WHERE excluded.\"firstOccurredAt\" < \"ContactActivity\".\"firstOccurredAt\"",
    )?;
    let mut n = 0;
    for r in records {
        n += stmt.execute(rusqlite::params![
            r.contact_id,
            r.activity_type,
            r.reference_type,
            r.parent_reference_id,
            r.reference_id,
            r.occurred_at,
            workspace_id
        ])?;
    }
    Ok(n)
}

/// `insertKeywordMessage`: idempotent by `(workspace, contact, message)`; `false` for a
/// duplicate or a contact of another workspace.
pub fn insert_keyword_message(conn: &Connection, workspace_id: i64, r: &ContactKeywordMessageRecord) -> Result<bool> {
    todo!("P3.2");
    let n = conn.execute(
        "INSERT INTO \"ContactKeywordMessage\" (\"workspaceId\", \"contactId\", \"messageId\", \"automatedResponseId\", \"text\", \"normalizedText\", \"occurredAt\") \
         SELECT c.\"workspaceId\", c.\"id\", ?2, ?3, ?4, ?5, ?6 FROM \"Contact\" AS c WHERE c.\"id\" = ?1 AND c.\"workspaceId\" = ?7 \
         ON CONFLICT (\"workspaceId\", \"contactId\", \"messageId\") DO NOTHING",
        rusqlite::params![r.contact_id, r.message_id, r.automated_response_id, r.text, r.normalized_text, r.occurred_at, workspace_id],
    )?;
    Ok(n > 0)
}

/// `upsertLastDelivered`: `lastDeliveredAt` only moves forward. Records are
/// `(contactId, deliveredAt)`.
pub fn upsert_last_delivered(conn: &Connection, workspace_id: i64, records: &[(i64, i64)]) -> Result<usize> {
    todo!("P3.2");
    let mut stmt = conn.prepare_cached(
        "INSERT INTO \"ContactState\" (\"workspaceId\", \"contactId\", \"lastDeliveredAt\") \
         SELECT c.\"workspaceId\", c.\"id\", ?2 FROM \"Contact\" AS c WHERE c.\"id\" = ?1 AND c.\"workspaceId\" = ?3 \
         ON CONFLICT (\"workspaceId\", \"contactId\") DO UPDATE SET \"lastDeliveredAt\" = excluded.\"lastDeliveredAt\" \
         WHERE \"ContactState\".\"lastDeliveredAt\" IS NULL OR excluded.\"lastDeliveredAt\" > \"ContactState\".\"lastDeliveredAt\"",
    )?;
    let mut n = 0;
    for (contact_id, delivered_at) in records {
        n += stmt.execute(rusqlite::params![contact_id, delivered_at, workspace_id])?;
    }
    Ok(n)
}

/// `deleteByContactIds`: removes the contacts' rows from the three tables.
pub fn delete_by_contact_ids(conn: &Connection, workspace_id: i64, contact_ids: &[i64]) -> Result<usize> {
    todo!("P3.2");
    let mut ids = contact_ids.to_vec();
    ids.sort_unstable();
    ids.dedup();
    if ids.is_empty() {
        return Ok(0);
    }
    let mut deleted = 0;
    for (table, _) in CONTACT_ACTIVITY_TABLE_KEYS {
        deleted += conn.execute(
            &format!("DELETE FROM \"{table}\" WHERE \"workspaceId\" = ?1 AND \"contactId\" IN (SELECT value FROM json_each(?2))"),
            rusqlite::params![workspace_id, id_list(&ids)],
        )?;
    }
    Ok(deleted)
}

/// `deleteWorkspaceBatch`: deletes up to `limit` rows (key order) of the workspace.
pub async fn delete_workspace_batch(db: &Db, table: &'static str, workspace_id: i64, limit: i64) -> Result<usize> {
    todo!("P3.2");
    let Some(key) = keys(table) else {
        return Err(super::Error::msg(format!("unknown contact activity table {table}")));
    };
    let cols = std::iter::once("\"workspaceId\"".to_owned())
        .chain(key.iter().map(|c| format!("\"{c}\"")))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "DELETE FROM \"{table}\" WHERE \"workspaceId\" = ?1 AND ({cols}) IN (SELECT {cols} FROM \"{table}\" WHERE \"workspaceId\" = ?1 ORDER BY {cols} LIMIT ?2)"
    );
    super::write(db, move |tx| Ok(tx.execute(&sql, rusqlite::params![workspace_id, limit])?)).await
}

/// `hasWorkspaceRows`.
pub fn has_workspace_rows(conn: &Connection, table: &str, workspace_id: i64) -> Result<bool> {
    todo!("P3.2");
    if keys(table).is_none() {
        return Err(super::Error::msg(format!("unknown contact activity table {table}")));
    }
    let mut stmt = conn.prepare_cached(&format!("SELECT 1 FROM \"{table}\" WHERE \"workspaceId\" = ?1 LIMIT 1"))?;
    Ok(stmt.exists([workspace_id])?)
}
