//! Port of `repositories/connection/repository.ts`.

use rusqlite::Connection as Conn;

use super::models::Connection;
use super::sql::Sql;
use super::utils::{PaginationInput, get_pagination_with_defaults};
use super::{Model, Result, text_list};

/// `ACTIVE_CONNECTION_STATUSES`.
pub const ACTIVE_CONNECTION_STATUSES: &[&str] = &["connected", "degraded"];

/// `ConnectionListInput`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConnectionListInput {
    pub workspace_id: i64,
    pub kind: Option<String>,
    pub provider: Option<String>,
    pub channel: Option<String>,
    pub status: Option<Vec<String>>,
    pub page: Option<i64>,
    pub per_page: Option<i64>,
}

fn build_where(input: &ConnectionListInput) -> Sql {
    Sql::and([
        Some(Sql::bind("\"workspaceId\" = ", input.workspace_id)),
        input.kind.clone().map(|k| Sql::bind("\"kind\" = ", k)),
        input
            .provider
            .clone()
            .map(|p| Sql::bind("\"provider\" = ", p)),
        input
            .channel
            .clone()
            .map(|c| Sql::bind("\"channel\" = ", c)),
        input.status.as_ref().filter(|s| !s.is_empty()).map(|s| {
            Sql::bind("\"status\" IN (SELECT value FROM json_each(", text_list(s)).closed("))")
        }),
    ])
}

/// `list`: one page, ordered by `(kind, provider, displayName, id)`.
pub fn list(conn: &Conn, input: &ConnectionListInput) -> Result<Vec<Connection>> {
    let p = get_pagination_with_defaults(PaginationInput {
        page: input.page,
        per_page: input.per_page,
    });
    let mut q = Sql::raw(format!("{} WHERE ", Connection::select_sql()));
    q.append(build_where(input));
    q.push(" ORDER BY \"kind\" ASC, \"provider\" ASC, \"displayName\" ASC, \"id\" ASC LIMIT ");
    q.push_bind(p.limit);
    q.push(" OFFSET ");
    q.push_bind(p.offset);
    q.models(conn)
}

/// `count`.
pub fn count(conn: &Conn, input: &ConnectionListInput) -> Result<i64> {
    let mut q = Sql::raw("SELECT count(*) FROM \"Connection\" WHERE ");
    q.append(build_where(input));
    Ok(q.query_map(conn, |r| r.get(0))?
        .into_iter()
        .next()
        .unwrap_or(0))
}

/// `findByIdForWorkspace`.
pub fn find_by_id_for_workspace(
    conn: &Conn,
    id: i64,
    workspace_id: i64,
) -> Result<Option<Connection>> {
    super::find_one(
        conn,
        "\"id\" = ?1 AND \"workspaceId\" = ?2",
        &[&id, &workspace_id],
    )
}

/// `findByProviderSourceId`.
pub fn find_by_provider_source_id(
    conn: &Conn,
    workspace_id: i64,
    provider: &str,
    source_id: &str,
) -> Result<Option<Connection>> {
    super::find_one(
        conn,
        "\"workspaceId\" = ?1 AND \"provider\" = ?2 AND \"sourceId\" = ?3",
        &[&workspace_id, &provider, &source_id],
    )
}

/// `findByProviderAndSourceIdAnyWorkspace`: an active match first, then the newest.
pub fn find_by_provider_and_source_id_any_workspace(
    conn: &Conn,
    provider: &str,
    source_id: &str,
) -> Result<Option<Connection>> {
    super::find_one(
        conn,
        "\"provider\" = ?1 AND \"sourceId\" = ?2 \
         ORDER BY CASE WHEN \"status\" IN ('connected', 'degraded') THEN 0 ELSE 1 END, \"id\" DESC",
        &[&provider, &source_id],
    )
}

/// `findByProviderAndSourceIdsAnyWorkspace`: by `sourceId`, then active first, newest.
pub fn find_by_provider_and_source_ids_any_workspace(
    conn: &Conn,
    provider: &str,
    source_ids: &[String],
) -> Result<Vec<Connection>> {
    if source_ids.is_empty() {
        return Ok(Vec::new());
    }
    super::find_many(
        conn,
        "WHERE \"provider\" = ?1 AND \"sourceId\" IN (SELECT value FROM json_each(?2)) \
         ORDER BY \"sourceId\", CASE WHEN \"status\" IN ('connected', 'degraded') THEN 0 ELSE 1 END, \"id\" DESC",
        &[&provider, &text_list(source_ids)],
    )
}

/// `findById` (also `findByIdForUpdateById`: no row locks in SQLite).
pub fn find_by_id(conn: &Conn, id: i64) -> Result<Option<Connection>> {
    super::find_one(conn, "\"id\" = ?1", &[&id])
}

/// `findByInboxId`.
pub fn find_by_inbox_id(conn: &Conn, inbox_id: i64) -> Result<Option<Connection>> {
    super::find_one(conn, "\"inboxId\" = ?1", &[&inbox_id])
}

/// `findByIntegrationId`.
pub fn find_by_integration_id(conn: &Conn, integration_id: i64) -> Result<Option<Connection>> {
    super::find_one(conn, "\"integrationId\" = ?1", &[&integration_id])
}

/// The columns of [`update`] (`None` = untouched; `Some(None)` = NULL).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConnectionPatch {
    pub display_name: Option<String>,
    pub status: Option<String>,
    pub status_reason: Option<Option<String>>,
    pub last_error: Option<Option<String>>,
    pub auth_expires_at: Option<Option<i64>>,
    pub connected_at: Option<Option<i64>>,
    pub disconnected_at: Option<Option<i64>>,
    pub provider_event_at: Option<Option<i64>>,
    pub provider_event_kind: Option<Option<String>>,
}

/// `update`: scoped to the workspace; `None` outside it.
pub fn update(
    conn: &Conn,
    id: i64,
    workspace_id: i64,
    values: &ConnectionPatch,
) -> Result<Option<Connection>> {
    use rusqlite::types::Value;
    let mut q = Sql::bind(
        "UPDATE \"Connection\" SET \"updatedAt\" = ",
        super::now_ms(),
    );
    let text = |v: &Option<String>| v.clone().map_or(Value::Null, Value::Text);
    let int = |v: Option<i64>| v.map_or(Value::Null, Value::Integer);
    if let Some(v) = &values.display_name {
        q.push(", \"displayName\" = ");
        q.push_bind(v.clone());
    }
    if let Some(v) = &values.status {
        q.push(", \"status\" = ");
        q.push_bind(v.clone());
    }
    for (col, v) in [
        ("statusReason", &values.status_reason),
        ("lastError", &values.last_error),
        ("providerEventKind", &values.provider_event_kind),
    ] {
        if let Some(v) = v {
            q.push(&format!(", \"{col}\" = "));
            q.push_bind(text(v));
        }
    }
    for (col, v) in [
        ("authExpiresAt", values.auth_expires_at),
        ("connectedAt", values.connected_at),
        ("disconnectedAt", values.disconnected_at),
        ("providerEventAt", values.provider_event_at),
    ] {
        if let Some(v) = v {
            q.push(&format!(", \"{col}\" = "));
            q.push_bind(int(v));
        }
    }
    q.push(" WHERE \"id\" = ");
    q.push_bind(id);
    q.push(" AND \"workspaceId\" = ");
    q.push_bind(workspace_id);
    q.push(&format!(" RETURNING {}", Connection::columns_sql(None)));
    q.model(conn)
}

/// `distinctProvidersByStatus`.
pub fn distinct_providers_by_status(
    conn: &Conn,
    workspace_id: i64,
    statuses: &[&str],
) -> Result<Vec<String>> {
    let mut stmt = conn.prepare_cached(
        "SELECT DISTINCT \"provider\" FROM \"Connection\" WHERE \"workspaceId\" = ?1 AND \"status\" IN (SELECT value FROM json_each(?2))",
    )?;
    let rows = stmt
        .query_map(rusqlite::params![workspace_id, text_list(statuses)], |r| {
            r.get(0)
        })?
        .collect::<rusqlite::Result<Vec<String>>>()?;
    Ok(rows)
}

/// `listPausedByOwner`: paused connections across the owner's workspaces.
pub fn list_paused_by_owner(conn: &Conn, owner_id: i64) -> Result<Vec<Connection>> {
    let sql = format!(
        "SELECT {} FROM \"Connection\" AS c INNER JOIN \"Workspace\" AS w ON c.\"workspaceId\" = w.\"id\" \
         WHERE w.\"ownerId\" = ?1 AND c.\"status\" = 'paused'",
        Connection::columns_sql(Some("c"))
    );
    super::query_models(conn, &sql, &[&owner_id])
}
