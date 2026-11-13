//! Port of `repositories/integration-messenger/repository.ts`
//! (`integrationMessengerRepository`, P3.2b).
//!
//! Every update also writes `updatedAt` (the Drizzle `sharedColumns` `$onUpdate`).

use rusqlite::Connection;
use rusqlite::types::Value;

use super::models::IntegrationMessenger;
use super::sql::{Sql, opt_int, opt_text};
use super::{Json, Model, Result, now_ms};

/// One row of `listByWorkspaceId`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessengerPageRef {
    pub id: i64,
    pub name: String,
    pub page_id: String,
}

/// One row of `listPersonasByWorkspaceId`.
#[derive(Debug, Clone, PartialEq)]
pub struct MessengerPersonas {
    pub name: String,
    pub personas: Json,
}

/// One row of `listConnectedForWebhookSubscription`.
#[derive(Debug, Clone, PartialEq)]
pub struct MessengerWebhookTarget {
    pub id: i64,
    pub workspace_id: i64,
    pub page_id: String,
    pub auth: Json,
}

/// `listByWorkspaceId`: `id`, `name`, `pageId` of the workspace's pages, by `createdAt`.
pub fn list_by_workspace_id(conn: &Connection, workspace_id: i64) -> Result<Vec<MessengerPageRef>> {
    Sql::bind(
        "SELECT \"id\", \"name\", \"pageId\" FROM \"IntegrationMessenger\" WHERE \"workspaceId\" = ",
        workspace_id,
    )
    .closed(" ORDER BY \"createdAt\", \"id\"")
    .query_map(conn, |r| {
        Ok(MessengerPageRef {
            id: r.get(0)?,
            name: r.get(1)?,
            page_id: r.get(2)?,
        })
    })
}

/// `listPersonasByWorkspaceId`: `name` and `personas` of the workspace's pages, by `createdAt`.
pub fn list_personas_by_workspace_id(
    conn: &Connection,
    workspace_id: i64,
) -> Result<Vec<MessengerPersonas>> {
    Sql::bind(
        "SELECT \"name\", \"personas\" FROM \"IntegrationMessenger\" WHERE \"workspaceId\" = ",
        workspace_id,
    )
    .closed(" ORDER BY \"createdAt\", \"id\"")
    .query_map(conn, |r| {
        Ok(MessengerPersonas {
            name: r.get(0)?,
            personas: r.get(1)?,
        })
    })
}

/// `findWorkspaceIntegration`.
pub fn find_workspace_integration(
    conn: &Connection,
    id: i64,
    workspace_id: i64,
) -> Result<Option<IntegrationMessenger>> {
    super::find_one(
        conn,
        "\"id\" = ?1 AND \"workspaceId\" = ?2",
        &[&id, &workspace_id],
    )
}

/// `findWorkspaceIntegrationByInboxId`: the integration id.
pub fn find_workspace_integration_by_inbox_id(
    conn: &Connection,
    workspace_id: i64,
    inbox_id: i64,
) -> Result<Option<i64>> {
    Ok(Sql::new(
        "SELECT \"id\" FROM \"IntegrationMessenger\" WHERE \"inboxId\" = ? AND \"workspaceId\" = ? LIMIT 1",
        vec![Value::Integer(inbox_id), Value::Integer(workspace_id)],
    )
    .query_map(conn, |r| r.get(0))?
    .into_iter()
    .next())
}

/// `UPDATE .. SET <set>, "updatedAt" = now WHERE id AND workspaceId [AND <extra>] RETURNING *`.
fn update(
    conn: &Connection,
    id: i64,
    workspace_id: i64,
    set: Vec<(&str, Value)>,
    extra: Option<Sql>,
) -> Result<Option<IntegrationMessenger>> {
    let mut q = Sql::raw("UPDATE \"IntegrationMessenger\" SET ");
    for (col, value) in set {
        q.push(&format!("\"{col}\" = "));
        q.push_bind(value);
        q.push(", ");
    }
    q.push("\"updatedAt\" = ");
    q.push_bind(now_ms());
    q.push(" WHERE \"id\" = ");
    q.push_bind(id);
    q.push(" AND \"workspaceId\" = ");
    q.push_bind(workspace_id);
    if let Some(extra) = extra {
        q.push(" AND ");
        q.append(extra);
    }
    q.push(&format!(
        " RETURNING {}",
        IntegrationMessenger::columns_sql(None)
    ));
    q.model(conn)
}

/// `capiScopeCheckedAt IS NOT DISTINCT FROM expected`.
fn capi_scope_cas(expected: Option<i64>) -> Sql {
    Sql::bind("\"capiScopeCheckedAt\" IS ", opt_int(expected))
}

/// `updateCapiScopeCache`: compare-and-set on `capiScopeCheckedAt`; on a lost race, the
/// current row.
pub fn update_capi_scope_cache(
    conn: &Connection,
    id: i64,
    workspace_id: i64,
    has_capi_scope: bool,
    capi_scope_checked_at: Option<i64>,
    expected_capi_scope_checked_at: Option<i64>,
) -> Result<Option<IntegrationMessenger>> {
    let set = vec![
        ("hasCapiScope", Value::Integer(i64::from(has_capi_scope))),
        ("capiScopeCheckedAt", opt_int(capi_scope_checked_at)),
    ];
    match update(
        conn,
        id,
        workspace_id,
        set,
        Some(capi_scope_cas(expected_capi_scope_checked_at)),
    )? {
        Some(row) => Ok(Some(row)),
        None => find_workspace_integration(conn, id, workspace_id),
    }
}

/// `claimCapiScopeCacheRefresh`: compare-and-set of `capiScopeCheckedAt` only.
pub fn claim_capi_scope_cache_refresh(
    conn: &Connection,
    id: i64,
    workspace_id: i64,
    capi_scope_checked_at: i64,
    expected_capi_scope_checked_at: Option<i64>,
) -> Result<Option<IntegrationMessenger>> {
    let set = vec![("capiScopeCheckedAt", Value::Integer(capi_scope_checked_at))];
    update(
        conn,
        id,
        workspace_id,
        set,
        Some(capi_scope_cas(expected_capi_scope_checked_at)),
    )
}

/// `updateDatasetIdIfNull`.
pub fn update_dataset_id_if_null(
    conn: &Connection,
    id: i64,
    workspace_id: i64,
    dataset_id: &str,
) -> Result<Option<IntegrationMessenger>> {
    let set = vec![("datasetId", Value::Text(dataset_id.to_owned()))];
    update(
        conn,
        id,
        workspace_id,
        set,
        Some(Sql::raw("\"datasetId\" IS NULL")),
    )
}

/// `updateDatasetId`.
pub fn update_dataset_id(
    conn: &Connection,
    id: i64,
    workspace_id: i64,
    dataset_id: &str,
) -> Result<Option<IntegrationMessenger>> {
    update(
        conn,
        id,
        workspace_id,
        vec![("datasetId", Value::Text(dataset_id.to_owned()))],
        None,
    )
}

/// `updateCapiTestEventCode` (`None` clears it).
pub fn update_capi_test_event_code(
    conn: &Connection,
    id: i64,
    workspace_id: i64,
    capi_test_event_code: Option<&str>,
) -> Result<Option<IntegrationMessenger>> {
    update(
        conn,
        id,
        workspace_id,
        vec![("capiTestEventCode", opt_text(capi_test_event_code))],
        None,
    )
}

/// `updateCapiAccessToken` (`EncryptedData` as JSON).
pub fn update_capi_access_token(
    conn: &Connection,
    id: i64,
    workspace_id: i64,
    capi_access_token: &Json,
) -> Result<Option<IntegrationMessenger>> {
    let set = vec![(
        "capiAccessToken",
        Value::Text(capi_access_token.0.to_string()),
    )];
    update(conn, id, workspace_id, set, None)
}

/// `connectCustomCapi`: dataset + token, and clears `capiDisconnectedAt`.
pub fn connect_custom_capi(
    conn: &Connection,
    id: i64,
    workspace_id: i64,
    dataset_id: &str,
    capi_access_token: &Json,
) -> Result<Option<IntegrationMessenger>> {
    let set = vec![
        ("datasetId", Value::Text(dataset_id.to_owned())),
        (
            "capiAccessToken",
            Value::Text(capi_access_token.0.to_string()),
        ),
        ("capiDisconnectedAt", Value::Null),
    ];
    update(conn, id, workspace_id, set, None)
}

/// `setCapiDisconnectedAt`: stamps the disconnect and drops the token.
pub fn set_capi_disconnected_at(
    conn: &Connection,
    id: i64,
    workspace_id: i64,
    capi_disconnected_at: i64,
) -> Result<Option<IntegrationMessenger>> {
    let set = vec![
        ("capiDisconnectedAt", Value::Integer(capi_disconnected_at)),
        ("capiAccessToken", Value::Null),
    ];
    update(conn, id, workspace_id, set, None)
}

/// `clearCapiDisconnectedAt`.
pub fn clear_capi_disconnected_at(
    conn: &Connection,
    id: i64,
    workspace_id: i64,
) -> Result<Option<IntegrationMessenger>> {
    update(
        conn,
        id,
        workspace_id,
        vec![("capiDisconnectedAt", Value::Null)],
        None,
    )
}

/// `clearCapiAccessToken`.
pub fn clear_capi_access_token(
    conn: &Connection,
    id: i64,
    workspace_id: i64,
) -> Result<Option<IntegrationMessenger>> {
    update(
        conn,
        id,
        workspace_id,
        vec![("capiAccessToken", Value::Null)],
        None,
    )
}

/// `findById` (unscoped).
pub fn find_by_id(conn: &Connection, id: i64) -> Result<Option<IntegrationMessenger>> {
    super::find_one(conn, "\"id\" = ?1", &[&id])
}

/// `findByPageIdUnscoped`.
pub fn find_by_page_id_unscoped(
    conn: &Connection,
    page_id: &str,
) -> Result<Option<IntegrationMessenger>> {
    super::find_one(conn, "\"pageId\" = ?1 ORDER BY \"id\"", &[&page_id])
}

/// `findByInboxId`.
pub fn find_by_inbox_id(conn: &Connection, inbox_id: i64) -> Result<Option<IntegrationMessenger>> {
    super::find_one(conn, "\"inboxId\" = ?1 ORDER BY \"id\"", &[&inbox_id])
}

/// `listByWorkspace` (by id).
pub fn list_by_workspace(
    conn: &Connection,
    workspace_id: i64,
) -> Result<Vec<IntegrationMessenger>> {
    super::find_many(
        conn,
        "WHERE \"workspaceId\" = ?1 ORDER BY \"id\"",
        &[&workspace_id],
    )
}

/// `updateHandoverResumeFlow` (`None` clears the flow); `None` when the integration is
/// not in the workspace.
pub fn update_handover_resume_flow(
    conn: &Connection,
    id: i64,
    workspace_id: i64,
    handover_resume_flow_id: Option<i64>,
) -> Result<Option<IntegrationMessenger>> {
    super::integration_whatsapp::update_flow(
        "IntegrationMessenger",
        conn,
        id,
        workspace_id,
        handover_resume_flow_id,
    )
}

/// `listConnectedForWebhookSubscription`: pages whose inbox is `connected`, keyset by id.
pub fn list_connected_for_webhook_subscription(
    conn: &Connection,
    after_id: Option<i64>,
    limit: i64,
) -> Result<Vec<MessengerWebhookTarget>> {
    let mut q = Sql::raw(
        "SELECT m.\"id\", m.\"workspaceId\", m.\"pageId\", m.\"auth\" FROM \"IntegrationMessenger\" AS m \
         INNER JOIN \"Inbox\" AS i ON i.\"id\" = m.\"inboxId\" WHERE i.\"status\" = 'connected'",
    );
    if let Some(after) = after_id {
        q.push(" AND m.\"id\" > ");
        q.push_bind(after);
    }
    q.push(" ORDER BY m.\"id\" LIMIT ");
    q.push_bind(limit);
    q.query_map(conn, |r| {
        Ok(MessengerWebhookTarget {
            id: r.get(0)?,
            workspace_id: r.get(1)?,
            page_id: r.get(2)?,
            auth: r.get(3)?,
        })
    })
}
