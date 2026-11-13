//! Port of `repositories/integration-whatsapp/repository.ts`
//! (`integrationWhatsappRepository`, P3.2b).

use rusqlite::Connection;
use rusqlite::types::Value;

use super::Result;
use super::sql::Sql;

/// `{ id, inboxId }` of `listByDisplayPhoneDigits`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IntegrationWhatsappInboxRef {
    pub id: i64,
    pub inbox_id: i64,
}

/// `listByDisplayPhoneDigits`: integrations of the workspace whose display phone number,
/// digits only, equals `digits` (at most two; an empty `digits` matches nothing).
pub fn list_by_display_phone_digits(
    conn: &Connection,
    workspace_id: i64,
    digits: &str,
) -> Result<Vec<IntegrationWhatsappInboxRef>> {
    if digits.is_empty() {
        return Ok(Vec::new());
    }
    Sql::new(
        "SELECT \"id\", \"inboxId\" FROM \"IntegrationWhatsapp\" WHERE \"workspaceId\" = ? \
         AND regexp_replace(\"displayPhoneNumber\", '\\D', '', 'g') = ? ORDER BY \"id\" LIMIT 2",
        vec![Value::Integer(workspace_id), Value::Text(digits.to_owned())],
    )
    .query_map(conn, |r| {
        Ok(IntegrationWhatsappInboxRef {
            id: r.get(0)?,
            inbox_id: r.get(1)?,
        })
    })
}

/// One row of `findAllConnectedForWebhookSubscription`.
#[derive(Debug, Clone, PartialEq)]
pub struct WebhookSubscriptionRow {
    pub id: i64,
    pub workspace_id: i64,
    pub waba_id: String,
    pub auth: super::Json,
}

/// `updateHandoverResumeFlow` (`None` clears the flow); `None` when the integration is
/// not in the workspace.
pub fn update_handover_resume_flow(
    conn: &Connection,
    id: i64,
    workspace_id: i64,
    handover_resume_flow_id: Option<i64>,
) -> Result<Option<super::models::IntegrationWhatsapp>> {
    update_flow(
        "IntegrationWhatsapp",
        conn,
        id,
        workspace_id,
        handover_resume_flow_id,
    )
}

/// `findAllConnectedForWebhookSubscription`: integrations whose inbox is `connected`
/// (every workspace, by id).
pub fn find_all_connected_for_webhook_subscription(
    conn: &Connection,
) -> Result<Vec<WebhookSubscriptionRow>> {
    Sql::raw(
        "SELECT w.\"id\", w.\"workspaceId\", w.\"wabaId\", w.\"auth\" FROM \"IntegrationWhatsapp\" AS w \
         INNER JOIN \"Inbox\" AS i ON i.\"id\" = w.\"inboxId\" WHERE i.\"status\" = 'connected' ORDER BY w.\"id\"",
    )
    .query_map(conn, |r| {
        Ok(WebhookSubscriptionRow {
            id: r.get(0)?,
            workspace_id: r.get(1)?,
            waba_id: r.get(2)?,
            auth: r.get(3)?,
        })
    })
}

/// `updateHandoverResumeFlow` of an integration table (scoped by id and workspace).
pub(crate) fn update_flow<M: super::Model>(
    table: &str,
    conn: &Connection,
    id: i64,
    workspace_id: i64,
    flow_id: Option<i64>,
) -> Result<Option<M>> {
    Sql::new(
        format!(
            "UPDATE \"{table}\" SET \"handoverResumeFlowId\" = ?, \"updatedAt\" = ? \
             WHERE \"id\" = ? AND \"workspaceId\" = ? RETURNING {}",
            M::columns_sql(None)
        ),
        vec![
            super::sql::opt_int(flow_id),
            Value::Integer(super::now_ms()),
            Value::Integer(id),
            Value::Integer(workspace_id),
        ],
    )
    .model(conn)
}
