//! Port of `repositories/integration-whatsapp/repository.ts`
//! (`integrationWhatsappRepository`, P3.2b).

use rusqlite::Connection;
use rusqlite::types::Value;

use super::models::IntegrationWhatsapp;
use super::patch::{Patch, set_clause};
use super::sql::{Sql, opt_int, opt_text};
use super::{Json, Model, Result};

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

// --- The untested rest of `integrationWhatsappRepository` (P3.2b, implementer 7). Every
// update also writes `updatedAt` (the Drizzle `sharedColumns` `$onUpdate`).

/// `UPDATE .. SET <set>, "updatedAt" = now WHERE <where> RETURNING *`.
fn update_where(
    conn: &Connection,
    set: &Patch,
    where_sql: Sql,
) -> Result<Option<IntegrationWhatsapp>> {
    let mut all = set.clone();
    all.push(("updatedAt", Value::Integer(super::now_ms())));
    let mut q = Sql::raw("UPDATE \"IntegrationWhatsapp\" SET ");
    q.append(set_clause(IntegrationWhatsapp::COLUMNS, &all)?);
    q.push(" WHERE ");
    q.append(where_sql);
    q.push(&format!(
        " RETURNING {}",
        IntegrationWhatsapp::columns_sql(None)
    ));
    q.model(conn)
}

/// `workspaceIntegrationFilter` (`id` and `workspaceId`) plus `extra`.
fn ws_filter(id: i64, workspace_id: i64, extra: Option<Sql>) -> Sql {
    let mut q = Sql::bind("\"id\" = ", id);
    q.push(" AND \"workspaceId\" = ");
    q.push_bind(workspace_id);
    if let Some(extra) = extra {
        q.push(" AND ");
        q.append(extra);
    }
    q
}

/// An update scoped by id and workspace.
fn update(
    conn: &Connection,
    id: i64,
    workspace_id: i64,
    set: &Patch,
    extra: Option<Sql>,
) -> Result<Option<IntegrationWhatsapp>> {
    update_where(conn, set, ws_filter(id, workspace_id, extra))
}

fn json_value(value: &Json) -> Value {
    super::patch::json(&value.0)
}

/// `findConnectedPhoneNumberIds`: the given phone number ids that have an integration.
pub fn find_connected_phone_number_ids(
    conn: &Connection,
    phone_number_ids: &[String],
) -> Result<std::collections::BTreeSet<String>> {
    if phone_number_ids.is_empty() {
        return Ok(std::collections::BTreeSet::new());
    }
    Ok(Sql::bind(
        "SELECT \"phoneNumberId\" FROM \"IntegrationWhatsapp\" WHERE \"phoneNumberId\" IN \
         (SELECT value FROM json_each(",
        super::text_list(phone_number_ids),
    )
    .closed("))")
    .query_map(conn, |r| r.get(0))?
    .into_iter()
    .collect())
}

/// `findAllForTokenRefresh`: `{ id, workspaceId, wabaId, auth }` of every integration, by id.
pub fn find_all_for_token_refresh(conn: &Connection) -> Result<Vec<WebhookSubscriptionRow>> {
    Sql::raw(
        "SELECT \"id\", \"workspaceId\", \"wabaId\", \"auth\" FROM \"IntegrationWhatsapp\" ORDER BY \"id\"",
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

/// One row of `findForTokenRefreshByWorkspaceIds`.
#[derive(Debug, Clone, PartialEq)]
pub struct TokenRefreshRow {
    pub id: i64,
    pub workspace_id: i64,
    pub auth: Json,
}

/// `findForTokenRefreshByWorkspaceIds`: `{ id, workspaceId, auth }` of the workspaces'
/// integrations, by id (empty input: none).
pub fn find_for_token_refresh_by_workspace_ids(
    conn: &Connection,
    workspace_ids: &[i64],
) -> Result<Vec<TokenRefreshRow>> {
    if workspace_ids.is_empty() {
        return Ok(Vec::new());
    }
    Sql::bind(
        "SELECT \"id\", \"workspaceId\", \"auth\" FROM \"IntegrationWhatsapp\" WHERE \
         \"workspaceId\" IN (SELECT value FROM json_each(",
        super::id_list(workspace_ids),
    )
    .closed(")) ORDER BY \"id\"")
    .query_map(conn, |r| {
        Ok(TokenRefreshRow {
            id: r.get(0)?,
            workspace_id: r.get(1)?,
            auth: r.get(2)?,
        })
    })
}

/// `findByIdForWorkspace`.
pub fn find_by_id_for_workspace(
    conn: &Connection,
    id: i64,
    workspace_id: i64,
) -> Result<Option<IntegrationWhatsapp>> {
    super::find_one(
        conn,
        "\"id\" = ?1 AND \"workspaceId\" = ?2",
        &[&id, &workspace_id],
    )
}

/// `findByIdUnscoped` (inbound webhooks that know only the id).
pub fn find_by_id_unscoped(conn: &Connection, id: i64) -> Result<Option<IntegrationWhatsapp>> {
    super::find_one(conn, "\"id\" = ?1", &[&id])
}

/// `updateAuth`: replaces `auth`, clears `tokenRefreshError`; the row's `phoneNumberId`.
pub fn update_auth(
    conn: &Connection,
    id: i64,
    workspace_id: i64,
    auth: &Json,
) -> Result<Option<String>> {
    let set = vec![
        ("auth", json_value(auth)),
        ("tokenRefreshError", Value::Null),
    ];
    Ok(update(conn, id, workspace_id, &set, None)?.map(|row| row.phone_number_id))
}

/// `updateAuthUnscoped`.
pub fn update_auth_unscoped(conn: &Connection, id: i64, auth: &Json) -> Result<()> {
    update_where(
        conn,
        &vec![("auth", json_value(auth))],
        Sql::bind("\"id\" = ", id),
    )?;
    Ok(())
}

/// `{ id, wabaId }` of `findWorkspaceIntegrationByInboxId`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntegrationWabaRef {
    pub id: i64,
    pub waba_id: String,
}

/// `inboxId = ? AND workspaceId = ?`.
const BY_INBOX: &str = "\"inboxId\" = ?1 AND \"workspaceId\" = ?2";

/// `findWorkspaceIntegrationByInboxId`.
pub fn find_workspace_integration_by_inbox_id(
    conn: &Connection,
    workspace_id: i64,
    inbox_id: i64,
) -> Result<Option<IntegrationWabaRef>> {
    Ok(
        find_by_inbox_id_for_workspace(conn, workspace_id, inbox_id)?.map(|row| {
            IntegrationWabaRef {
                id: row.id,
                waba_id: row.waba_id,
            }
        }),
    )
}

/// `findAuthByInboxIdForWorkspace`: the integration's `auth`.
pub fn find_auth_by_inbox_id_for_workspace(
    conn: &Connection,
    workspace_id: i64,
    inbox_id: i64,
) -> Result<Option<Json>> {
    Ok(find_by_inbox_id_for_workspace(conn, workspace_id, inbox_id)?.map(|row| row.auth))
}

/// `findByInboxIdForWorkspace`: the full row.
pub fn find_by_inbox_id_for_workspace(
    conn: &Connection,
    workspace_id: i64,
    inbox_id: i64,
) -> Result<Option<IntegrationWhatsapp>> {
    super::find_one(conn, BY_INBOX, &[&inbox_id, &workspace_id])
}

/// `markHistoryDeclined` (unscoped).
pub fn mark_history_declined(conn: &Connection, id: i64) -> Result<()> {
    update_where(
        conn,
        &vec![("historyDeclined", Value::Integer(1))],
        Sql::bind("\"id\" = ", id),
    )?;
    Ok(())
}

/// `findByPhoneNumberId`; `waba_id` (when set and not empty) narrows the match. The
/// lowest id wins.
pub fn find_by_phone_number_id(
    conn: &Connection,
    phone_number_id: &str,
    waba_id: Option<&str>,
) -> Result<Option<IntegrationWhatsapp>> {
    match waba_id.filter(|w| !w.is_empty()) {
        Some(waba) => super::find_one(
            conn,
            "\"phoneNumberId\" = ?1 AND \"wabaId\" = ?2 ORDER BY \"id\"",
            &[&phone_number_id, &waba],
        ),
        None => super::find_one(
            conn,
            "\"phoneNumberId\" = ?1 ORDER BY \"id\"",
            &[&phone_number_id],
        ),
    }
}

/// `{ id, name }` of an inbox.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InboxName {
    pub id: i64,
    pub name: String,
}

/// One row of `listByWorkspaceId`.
#[derive(Debug, Clone, PartialEq)]
pub struct IntegrationWhatsappWithInbox {
    pub integration: IntegrationWhatsapp,
    pub inbox: Option<InboxName>,
}

/// `listByWorkspaceId`: by `createdAt`, then id, each with its inbox `{ id, name }`.
pub fn list_by_workspace_id(
    conn: &Connection,
    workspace_id: i64,
) -> Result<Vec<IntegrationWhatsappWithInbox>> {
    let rows: Vec<IntegrationWhatsapp> = super::find_many(
        conn,
        "WHERE \"workspaceId\" = ?1 ORDER BY \"createdAt\" ASC, \"id\" ASC",
        &[&workspace_id],
    )?;
    let mut out = Vec::with_capacity(rows.len());
    for integration in rows {
        let inbox = Sql::bind(
            "SELECT \"id\", \"name\" FROM \"Inbox\" WHERE \"id\" = ",
            integration.inbox_id,
        )
        .query_map(conn, |r| {
            Ok(InboxName {
                id: r.get(0)?,
                name: r.get(1)?,
            })
        })?
        .into_iter()
        .next();
        out.push(IntegrationWhatsappWithInbox { integration, inbox });
    }
    Ok(out)
}

/// `findVerificationCodeRequestedAt`: `None` when the integration is not in the
/// workspace, else its `verificationCodeRequestedAt`.
pub fn find_verification_code_requested_at(
    conn: &Connection,
    id: i64,
    workspace_id: i64,
) -> Result<Option<Option<i64>>> {
    Ok(find_by_id_for_workspace(conn, id, workspace_id)?
        .map(|row| row.verification_code_requested_at))
}

const REGISTRATION_COLUMNS: &[&str] = &["registrationStatus", "registrationError"];

/// `updateRegistration`: sets `registrationStatus` / `registrationError` (the patch keys;
/// others are refused); the stored `registrationError` (`None` also when the row is
/// missing).
pub fn update_registration(
    conn: &Connection,
    id: i64,
    workspace_id: i64,
    values: &Patch,
) -> Result<Option<Json>> {
    super::patch::check(REGISTRATION_COLUMNS, values)?;
    Ok(update(conn, id, workspace_id, values, None)?.and_then(|row| row.registration_error))
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
) -> Result<Option<IntegrationWhatsapp>> {
    let set = vec![
        ("hasCapiScope", Value::Integer(i64::from(has_capi_scope))),
        ("capiScopeCheckedAt", opt_int(capi_scope_checked_at)),
    ];
    let cas = capi_scope_cas(expected_capi_scope_checked_at);
    match update(conn, id, workspace_id, &set, Some(cas))? {
        Some(row) => Ok(Some(row)),
        None => find_by_id_for_workspace(conn, id, workspace_id),
    }
}

/// `claimCapiScopeCacheRefresh`: compare-and-set of `capiScopeCheckedAt` only.
pub fn claim_capi_scope_cache_refresh(
    conn: &Connection,
    id: i64,
    workspace_id: i64,
    capi_scope_checked_at: i64,
    expected_capi_scope_checked_at: Option<i64>,
) -> Result<Option<IntegrationWhatsapp>> {
    let set = vec![("capiScopeCheckedAt", Value::Integer(capi_scope_checked_at))];
    let cas = capi_scope_cas(expected_capi_scope_checked_at);
    update(conn, id, workspace_id, &set, Some(cas))
}

/// `replaceAuth`: `auth` with the CAPI scope cache.
pub fn replace_auth(
    conn: &Connection,
    id: i64,
    workspace_id: i64,
    auth: &Json,
    has_capi_scope: bool,
    capi_scope_checked_at: i64,
) -> Result<Option<IntegrationWhatsapp>> {
    let set = vec![
        ("auth", json_value(auth)),
        ("hasCapiScope", Value::Integer(i64::from(has_capi_scope))),
        ("capiScopeCheckedAt", Value::Integer(capi_scope_checked_at)),
    ];
    update(conn, id, workspace_id, &set, None)
}

/// `updateDatasetIdIfNull`.
pub fn update_dataset_id_if_null(
    conn: &Connection,
    id: i64,
    workspace_id: i64,
    dataset_id: &str,
) -> Result<Option<IntegrationWhatsapp>> {
    let set = vec![("datasetId", Value::Text(dataset_id.to_owned()))];
    let extra = Sql::raw("\"datasetId\" IS NULL");
    update(conn, id, workspace_id, &set, Some(extra))
}

/// `updateDatasetId` (unconditional).
pub fn update_dataset_id(
    conn: &Connection,
    id: i64,
    workspace_id: i64,
    dataset_id: &str,
) -> Result<Option<IntegrationWhatsapp>> {
    let set = vec![("datasetId", Value::Text(dataset_id.to_owned()))];
    update(conn, id, workspace_id, &set, None)
}

/// `updateCapiTestEventCode` (`None` clears it).
pub fn update_capi_test_event_code(
    conn: &Connection,
    id: i64,
    workspace_id: i64,
    capi_test_event_code: Option<&str>,
) -> Result<Option<IntegrationWhatsapp>> {
    let set = vec![("capiTestEventCode", opt_text(capi_test_event_code))];
    update(conn, id, workspace_id, &set, None)
}

/// `updateCapiAccessToken` (`EncryptedData` as JSON).
pub fn update_capi_access_token(
    conn: &Connection,
    id: i64,
    workspace_id: i64,
    capi_access_token: &Json,
) -> Result<Option<IntegrationWhatsapp>> {
    let set = vec![("capiAccessToken", json_value(capi_access_token))];
    update(conn, id, workspace_id, &set, None)
}

/// `connectCustomCapi`: dataset + token, and clears `capiDisconnectedAt`.
pub fn connect_custom_capi(
    conn: &Connection,
    id: i64,
    workspace_id: i64,
    dataset_id: &str,
    capi_access_token: &Json,
) -> Result<Option<IntegrationWhatsapp>> {
    let set = vec![
        ("datasetId", Value::Text(dataset_id.to_owned())),
        ("capiAccessToken", json_value(capi_access_token)),
        ("capiDisconnectedAt", Value::Null),
    ];
    update(conn, id, workspace_id, &set, None)
}

/// `setCapiDisconnectedAt`: stamps the disconnect and drops the token.
pub fn set_capi_disconnected_at(
    conn: &Connection,
    id: i64,
    workspace_id: i64,
    capi_disconnected_at: i64,
) -> Result<Option<IntegrationWhatsapp>> {
    let set = vec![
        ("capiDisconnectedAt", Value::Integer(capi_disconnected_at)),
        ("capiAccessToken", Value::Null),
    ];
    update(conn, id, workspace_id, &set, None)
}

/// `clearCapiDisconnectedAt`.
pub fn clear_capi_disconnected_at(
    conn: &Connection,
    id: i64,
    workspace_id: i64,
) -> Result<Option<IntegrationWhatsapp>> {
    let set = vec![("capiDisconnectedAt", Value::Null)];
    update(conn, id, workspace_id, &set, None)
}

/// `clearCapiAccessToken`.
pub fn clear_capi_access_token(
    conn: &Connection,
    id: i64,
    workspace_id: i64,
) -> Result<Option<IntegrationWhatsapp>> {
    let set = vec![("capiAccessToken", Value::Null)];
    update(conn, id, workspace_id, &set, None)
}

/// `claimVerificationCodeSlot`: stamps `now` when the previous request is absent or older
/// than `cutoff`; the stamped time, or `None` when another claim holds the slot.
pub fn claim_verification_code_slot(
    conn: &Connection,
    id: i64,
    workspace_id: i64,
    now: i64,
    cutoff: i64,
) -> Result<Option<i64>> {
    let set = vec![("verificationCodeRequestedAt", Value::Integer(now))];
    let extra = Sql::bind(
        "(\"verificationCodeRequestedAt\" IS NULL OR \"verificationCodeRequestedAt\" < ",
        cutoff,
    )
    .closed(")");
    Ok(update(conn, id, workspace_id, &set, Some(extra))?
        .and_then(|row| row.verification_code_requested_at))
}

/// `releaseVerificationCodeSlot`: clears the stamp only while it is still `claimed_at`.
pub fn release_verification_code_slot(
    conn: &Connection,
    id: i64,
    workspace_id: i64,
    claimed_at: i64,
) -> Result<()> {
    let set = vec![("verificationCodeRequestedAt", Value::Null)];
    let extra = Sql::bind("\"verificationCodeRequestedAt\" = ", claimed_at);
    update(conn, id, workspace_id, &set, Some(extra))?;
    Ok(())
}

/// `isCallRecordingEnabledForInbox` (`false` without an integration).
pub fn is_call_recording_enabled_for_inbox(
    conn: &Connection,
    workspace_id: i64,
    inbox_id: i64,
) -> Result<bool> {
    Ok(
        find_by_inbox_id_for_workspace(conn, workspace_id, inbox_id)?
            .is_some_and(|row| row.call_recording_enabled),
    )
}

/// `updateCallRecordingEnabled`.
pub fn update_call_recording_enabled(
    conn: &Connection,
    id: i64,
    workspace_id: i64,
    enabled: bool,
) -> Result<()> {
    let set = vec![("callRecordingEnabled", Value::Integer(i64::from(enabled)))];
    update(conn, id, workspace_id, &set, None)?;
    Ok(())
}

/// `updatePixRedirectEnabled`; `None` when the number is not in the workspace.
pub fn update_pix_redirect_enabled(
    conn: &Connection,
    id: i64,
    workspace_id: i64,
    enabled: bool,
) -> Result<Option<IntegrationWhatsapp>> {
    let set = vec![("pixRedirectEnabled", Value::Integer(i64::from(enabled)))];
    update(conn, id, workspace_id, &set, None)
}

const CALL_SETTINGS_COLUMNS: &[&str] = &[
    "callRecordingEnabled",
    "callRecordingRetentionDays",
    "callTranscriptionEnabled",
    "callingEnabled",
    "inboundCallsEnabled",
    "callHours",
];

/// `updateCallSettings`: the patch keys of `UpdateCallSettingsInput.values` (others are
/// refused); with `only_while_recording`, matches only while `callRecordingEnabled`.
pub fn update_call_settings(
    conn: &Connection,
    id: i64,
    workspace_id: i64,
    values: &Patch,
    only_while_recording: bool,
) -> Result<Option<IntegrationWhatsapp>> {
    super::patch::check(CALL_SETTINGS_COLUMNS, values)?;
    let extra = only_while_recording.then(|| Sql::raw("\"callRecordingEnabled\" = 1"));
    update(conn, id, workspace_id, values, extra)
}

/// `updateRoutingRolesIfNewer`: stores `routing_roles` (a `RoutingRolesSnapshot`) unless
/// the stored snapshot's `checkedAt` is not older (text comparison, as the TS); `None`
/// when the row is missing or a newer snapshot kept it.
pub fn update_routing_roles_if_newer(
    conn: &Connection,
    id: i64,
    workspace_id: i64,
    routing_roles: &Json,
) -> Result<Option<IntegrationWhatsapp>> {
    let checked_at = routing_roles
        .0
        .get("checkedAt")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| super::Error::msg("routingRoles.checkedAt must be a string"))?
        .to_owned();
    let set = vec![("routingRoles", json_value(routing_roles))];
    let extra = Sql::bind(
        "COALESCE(\"routingRoles\" ->> '$.checkedAt', '') < ",
        checked_at,
    );
    update(conn, id, workspace_id, &set, Some(extra))
}
