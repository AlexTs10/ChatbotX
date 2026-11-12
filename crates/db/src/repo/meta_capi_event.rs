//! Port of `repositories/meta-capi-event/repository.ts` (P3.3).

use rusqlite::Connection;
use rusqlite::types::Value;

use super::models::MetaCapiEvent;
use super::patch::{Patch, insert_clause};
use super::sql::{Sql, opt_int, opt_text};
use super::{Model, Result, new_id, now_ms};

/// `insertIgnoreDuplicate`: `values` = the columns besides `id` (timestamps default to
/// now); `None` when `(workspaceId, channel, sourceKey)` already exists.
pub fn insert_ignore_duplicate(conn: &Connection, values: &Patch) -> Result<Option<MetaCapiEvent>> {
    let now = now_ms();
    let mut all: Patch = vec![("id", Value::Integer(new_id()))];
    for (col, default) in [("createdAt", now), ("updatedAt", now)] {
        if !values.iter().any(|(c, _)| *c == col) {
            all.push((col, Value::Integer(default)));
        }
    }
    all.extend(values.iter().cloned());
    let mut q = Sql::raw("INSERT INTO \"MetaCapiEvent\" ");
    q.append(insert_clause(MetaCapiEvent::COLUMNS, &all)?);
    q.push(" ON CONFLICT (\"workspaceId\", \"channel\", \"sourceKey\") DO NOTHING");
    q.push(&format!(" RETURNING {}", MetaCapiEvent::columns_sql(None)));
    q.model(conn)
}

/// `findWorkspaceEvent`.
pub fn find_workspace_event(
    conn: &Connection,
    id: i64,
    workspace_id: i64,
) -> Result<Option<MetaCapiEvent>> {
    super::find_one(
        conn,
        "\"id\" = ?1 AND \"workspaceId\" = ?2",
        &[&id, &workspace_id],
    )
}

/// `findPendingBySourceKey`.
pub fn find_pending_by_source_key(
    conn: &Connection,
    workspace_id: i64,
    channel: &str,
    source_key: &str,
) -> Result<Option<MetaCapiEvent>> {
    super::find_one(
        conn,
        "\"workspaceId\" = ?1 AND \"channel\" = ?2 AND \"sourceKey\" = ?3 AND \"capiStatus\" = 'pending'",
        &[&workspace_id, &channel, &source_key],
    )
}

/// `deleteByIntegration`.
pub fn delete_by_integration(
    conn: &Connection,
    workspace_id: i64,
    channel: &str,
    integration_id: i64,
) -> Result<()> {
    conn.execute(
        "DELETE FROM \"MetaCapiEvent\" WHERE \"workspaceId\" = ?1 AND \"channel\" = ?2 AND \"integrationId\" = ?3",
        rusqlite::params![workspace_id, channel, integration_id],
    )?;
    Ok(())
}

/// `updateCapiStatus`: CAS `from` -> `to`. A `None` `capi_sent_at` / `capi_error` is not
/// written (Drizzle drops `undefined`); `Some(None)` error writes NULL.
pub fn update_capi_status(
    conn: &Connection,
    id: i64,
    workspace_id: i64,
    from: &str,
    to: &str,
    capi_sent_at: Option<i64>,
    capi_error: Option<Option<&str>>,
) -> Result<Option<MetaCapiEvent>> {
    let mut q = Sql::bind(
        "UPDATE \"MetaCapiEvent\" SET \"capiStatus\" = ",
        to.to_owned(),
    );
    if let Some(at) = capi_sent_at {
        q.push(", \"capiSentAt\" = ");
        q.push_bind(opt_int(Some(at)));
    }
    if let Some(err) = capi_error {
        q.push(", \"capiError\" = ");
        q.push_bind(opt_text(err));
    }
    q.push(", \"updatedAt\" = ");
    q.push_bind(now_ms());
    q.push(" WHERE \"id\" = ");
    q.push_bind(id);
    q.push(" AND \"workspaceId\" = ");
    q.push_bind(workspace_id);
    q.push(" AND \"capiStatus\" = ");
    q.push_bind(from.to_owned());
    q.push(&format!(" RETURNING {}", MetaCapiEvent::columns_sql(None)));
    q.model(conn)
}
