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

/// `PROVIDER_ACCOUNT_TARGET_BATCH_SIZE` (`@chatbotx.io/utils` `connection.ts`).
pub const PROVIDER_ACCOUNT_TARGET_BATCH_SIZE: i64 = 200;

/// `ProviderAccountRouteScope`: which numbers a provider account event may touch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderAccountRouteScope {
    /// The platform app.
    Platform { app_id: String },
    /// A user- or reseller-owned app whose credential owner is `owner_id`.
    CredentialOwner { owner_id: i64, app_id: String },
    /// One manually connected integration.
    ManualIntegration {
        integration_id: i64,
        workspace_id: i64,
    },
}

/// `ProviderAccountTarget`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderAccountTarget {
    pub integration_id: i64,
    pub workspace_id: i64,
    pub workspace_owner_id: i64,
    pub inbox_id: i64,
    /// `None` for a row predating the Connection backfill.
    pub connection_id: Option<i64>,
}

/// `ConnectionStatusState` of a client resource inbox.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionStatusState {
    pub status: String,
    pub status_reason: Option<String>,
}

/// The `inbox` of an [`IntegrationWhatsappClientResource`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientResourceInbox {
    pub id: i64,
    pub name: String,
    pub connection: Option<ConnectionStatusState>,
}

/// `IntegrationWhatsappClientResource` (no `auth`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntegrationWhatsappClientResource {
    pub id: i64,
    pub name: String,
    pub inbox_id: i64,
    pub display_phone_number: String,
    pub token_refresh_error: Option<String>,
    pub phone_number_id: String,
    pub waba_id: String,
    pub has_capi_scope: bool,
    pub capi_scope_checked_at: Option<i64>,
    pub dataset_id: Option<String>,
    pub workspace_id: i64,
    pub created_at: i64,
    pub is_coexist: bool,
    pub is_manual: bool,
    pub inbox: Option<ClientResourceInbox>,
}

/// `isManualWhatsappAuth`: `COALESCE(auth #>> '{metadata,isManual}', 'false') = 'true'`
/// (a JSON `true` or the text `"true"`).
fn is_manual_sql(auth: &str) -> String {
    format!(
        "COALESCE(json_type({auth}, '$.metadata.isManual') = 'true' OR \
         (json_type({auth}, '$.metadata.isManual') = 'text' AND \
         json_extract({auth}, '$.metadata.isManual') = 'true'), 0)"
    )
}

/// `markTokenRefreshError`: scoped by id and workspace; the row's `phoneNumberId`.
pub fn mark_token_refresh_error(
    conn: &Connection,
    id: i64,
    workspace_id: i64,
    error: &str,
) -> Result<Option<String>> {
    let mut stmt = conn.prepare_cached(
        "UPDATE \"IntegrationWhatsapp\" SET \"tokenRefreshError\" = ?1, \"updatedAt\" = ?2 \
         WHERE \"id\" = ?3 AND \"workspaceId\" = ?4 RETURNING \"phoneNumberId\"",
    )?;
    let mut rows = stmt.query(rusqlite::params![error, super::now_ms(), id, workspace_id])?;
    Ok(match rows.next()? {
        Some(row) => Some(row.get(0)?),
        None => None,
    })
}

/// `listAccountTargetsByWabaId`: the numbers of `waba_id` that `scope` may act on, by id,
/// at most [`PROVIDER_ACCOUNT_TARGET_BATCH_SIZE`], after `after_integration_id`.
pub fn list_account_targets_by_waba_id(
    conn: &Connection,
    waba_id: &str,
    scope: &ProviderAccountRouteScope,
    after_integration_id: Option<i64>,
) -> Result<Vec<ProviderAccountTarget>> {
    // The tenant's owner for a reseller tenant, else the workspace's own owner.
    let owner = format!(
        "(CASE WHEN w.\"tenantId\" <> {root} THEN COALESCE(t.\"ownerId\", w.\"ownerId\") \
         ELSE w.\"ownerId\" END)",
        root = super::platform_admin::ROOT_TENANT_ID
    );
    let manual = is_manual_sql("iw.\"auth\"");
    let through_app = |app: &str, fallback: &str| {
        let mut s = Sql::bind("(iw.\"auth\" ->> '$.clientId' = ", app.to_owned());
        s.push(&format!(
            " OR (iw.\"auth\" ->> '$.clientId' IS NULL AND {fallback}))"
        ));
        s
    };
    let scope_sql = match scope {
        ProviderAccountRouteScope::ManualIntegration {
            integration_id,
            workspace_id,
        } => {
            let mut s = Sql::bind("iw.\"id\" = ", *integration_id);
            s.append(Sql::bind(" AND iw.\"workspaceId\" = ", *workspace_id));
            s
        }
        ProviderAccountRouteScope::CredentialOwner { owner_id, app_id } => {
            let mut s = Sql::bind(format!("{owner} = "), *owner_id);
            s.push(&format!(" AND NOT ({manual}) AND "));
            s.append(through_app(app_id, "1"));
            s
        }
        ProviderAccountRouteScope::Platform { app_id } => {
            let mut s = Sql::raw(format!("NOT ({manual}) AND "));
            s.append(through_app(
                app_id,
                &format!(
                    "NOT EXISTS (SELECT 1 FROM \"PlatformCredential\" AS pc WHERE pc.\"userId\" = \
                     {owner} AND pc.\"type\" = 'whatsapp' AND pc.\"usePlatformCredential\" = 0)"
                ),
            ));
            s
        }
    };
    let mut q = Sql::bind(
        "SELECT iw.\"id\", iw.\"workspaceId\", w.\"ownerId\", iw.\"inboxId\", c.\"id\" \
         FROM \"IntegrationWhatsapp\" AS iw \
         JOIN \"Workspace\" AS w ON w.\"id\" = iw.\"workspaceId\" \
         LEFT JOIN \"Tenant\" AS t ON t.\"id\" = w.\"tenantId\" \
         LEFT JOIN \"Connection\" AS c ON c.\"inboxId\" = iw.\"inboxId\" \
         WHERE iw.\"wabaId\" = ",
        waba_id.to_owned(),
    );
    if let Some(after) = after_integration_id {
        q.append(Sql::bind(" AND iw.\"id\" > ", after));
    }
    q.push(" AND (");
    q.append(scope_sql);
    q.push(") ORDER BY iw.\"id\" LIMIT ");
    q.push_bind(PROVIDER_ACCOUNT_TARGET_BATCH_SIZE);
    q.query_map(conn, |r| {
        Ok(ProviderAccountTarget {
            integration_id: r.get(0)?,
            workspace_id: r.get(1)?,
            workspace_owner_id: r.get(2)?,
            inbox_id: r.get(3)?,
            connection_id: r.get(4)?,
        })
    })
}

/// `listClientResourcesByWorkspaceId` (createdAt ASC, id ASC).
pub fn list_client_resources_by_workspace_id(
    conn: &Connection,
    workspace_id: i64,
) -> Result<Vec<IntegrationWhatsappClientResource>> {
    let sql = format!(
        "SELECT iw.\"id\", iw.\"name\", iw.\"inboxId\", iw.\"displayPhoneNumber\", \
         iw.\"tokenRefreshError\", iw.\"phoneNumberId\", iw.\"wabaId\", iw.\"hasCapiScope\", \
         iw.\"capiScopeCheckedAt\", iw.\"datasetId\", iw.\"workspaceId\", iw.\"createdAt\", \
         iw.\"isCoexist\", {manual}, i.\"id\", i.\"name\", c.\"status\", c.\"statusReason\" \
         FROM \"IntegrationWhatsapp\" AS iw \
         LEFT JOIN \"Inbox\" AS i ON i.\"id\" = iw.\"inboxId\" \
         LEFT JOIN \"Connection\" AS c ON c.\"inboxId\" = iw.\"inboxId\" \
         WHERE iw.\"workspaceId\" = ?1 ORDER BY iw.\"createdAt\" ASC, iw.\"id\" ASC",
        manual = is_manual_sql("iw.\"auth\"")
    );
    let mut stmt = conn.prepare_cached(&sql)?;
    let rows = stmt
        .query_map([workspace_id], |r| {
            let inbox_id: Option<i64> = r.get(14)?;
            let status: Option<String> = r.get(16)?;
            Ok(IntegrationWhatsappClientResource {
                id: r.get(0)?,
                name: r.get(1)?,
                inbox_id: r.get(2)?,
                display_phone_number: r.get(3)?,
                token_refresh_error: r.get(4)?,
                phone_number_id: r.get(5)?,
                waba_id: r.get(6)?,
                has_capi_scope: r.get(7)?,
                capi_scope_checked_at: r.get(8)?,
                dataset_id: r.get(9)?,
                workspace_id: r.get(10)?,
                created_at: r.get(11)?,
                is_coexist: r.get(12)?,
                is_manual: r.get(13)?,
                inbox: match inbox_id {
                    Some(id) => Some(ClientResourceInbox {
                        id,
                        name: r.get(15)?,
                        connection: match status {
                            Some(status) => Some(ConnectionStatusState {
                                status,
                                status_reason: r.get(17)?,
                            }),
                            None => None,
                        },
                    }),
                    None => None,
                },
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}
