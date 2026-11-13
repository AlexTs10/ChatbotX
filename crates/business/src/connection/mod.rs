//! `packages/business/src/connection` + `packages/connections/src` (the
//! self-serve part: `api` and `webchat` channels; P3.6-D2). The provider
//! catalog, credential / OAuth connects and connect sessions come later.

pub mod providers;

use chatbotx_db::Db;
use chatbotx_db::repo;
use chatbotx_db::rusqlite::{Connection, OptionalExtension as _, Row, params};

use crate::{BusinessError, Result, tx};

/// The `Connection` columns read by [`row`].
pub(crate) const COLUMNS: &str = r#""id", "workspaceId", "provider", "kind", "channel", "inboxId",
    "integrationId", "sourceId", "displayName", "status", "statusReason", "lastError",
    "authExpiresAt", "connectedAt", "disconnectedAt", "createdAt", "updatedAt""#;

/// One `Connection` row of a `SELECT` [`COLUMNS`].
pub(crate) fn row(r: &Row<'_>) -> chatbotx_db::rusqlite::Result<ConnectionRow> {
    Ok(ConnectionRow {
        id: r.get(0)?,
        workspace_id: r.get(1)?,
        provider: r.get(2)?,
        kind: r.get(3)?,
        channel: r.get(4)?,
        inbox_id: r.get(5)?,
        integration_id: r.get(6)?,
        source_id: r.get(7)?,
        display_name: r.get(8)?,
        status: r.get(9)?,
        status_reason: r.get(10)?,
        last_error: r.get(11)?,
        auth_expires_at: r.get(12)?,
        connected_at: r.get(13)?,
        disconnected_at: r.get(14)?,
        created_at: r.get(15)?,
        updated_at: r.get(16)?,
    })
}

fn repo_err(error: chatbotx_db::rusqlite::Error) -> BusinessError {
    BusinessError::Repo(error.into())
}

/// `connectionRepository.findByIdForWorkspace` (private query).
pub(crate) fn find(conn: &Connection, id: i64, workspace_id: i64) -> Result<Option<ConnectionRow>> {
    conn.query_row(
        &format!(r#"SELECT {COLUMNS} FROM "Connection" WHERE "id" = ?1 AND "workspaceId" = ?2"#),
        params![id, workspace_id],
        row,
    )
    .optional()
    .map_err(repo_err)
}

fn find_or_fail(conn: &Connection, id: i64, workspace_id: i64) -> Result<ConnectionRow> {
    find(conn, id, workspace_id)?.ok_or_else(|| BusinessError::not_found("Connection not found"))
}

/// A `connected` channel connection for a new inbox whose id is also the
/// source id (`upsertConnectionRow`
/// insert + `connect.completed`, one statement). A duplicate
/// `(workspaceId, provider, sourceId)` is `connectionAlreadyConnected`.
pub(crate) fn insert_connected_channel(
    conn: &Connection,
    workspace_id: i64,
    provider: &str,
    inbox_id: i64,
    display_name: &str,
    created_by: Option<i64>,
    now: i64,
) -> Result<ConnectionRow> {
    let id: i64 = chatbotx_core::utils::id::create_id()
        .parse()
        .unwrap_or_default();
    let channel = providers::to_channel_type(provider);
    let source_id = inbox_id.to_string();
    let inserted = conn.execute(
        r#"INSERT INTO "Connection" ("id", "createdAt", "updatedAt", "workspaceId", "provider",
               "kind", "channel", "inboxId", "sourceId", "displayName", "status", "createdBy",
               "connectedAt")
           VALUES (?1, ?2, ?2, ?3, ?4, 'channel', ?5, ?6, ?7, ?8, 'connected', ?9, ?2)"#,
        params![
            id,
            now,
            workspace_id,
            provider,
            channel,
            inbox_id,
            source_id,
            display_name,
            created_by
        ],
    );
    match inserted {
        Ok(_) => {}
        Err(error)
            if error
                .to_string()
                .contains("UNIQUE constraint failed: Connection") =>
        {
            return Err(BusinessError::exception(
                "This provider is already connected in this workspace.",
                "connectionAlreadyConnected",
                409,
            ));
        }
        Err(error) => return Err(repo_err(error)),
    }
    find_or_fail(conn, id, workspace_id)
}

/// `inboxService.create` (`skipQuota`; private query): a `connected` inbox
/// whose id is also its source id.
pub(crate) fn insert_inbox(
    conn: &Connection,
    id: i64,
    workspace_id: i64,
    channel: &str,
    name: &str,
    now: i64,
) -> Result<()> {
    conn.execute(
        r#"INSERT INTO "Inbox" ("id", "createdAt", "updatedAt", "name", "channel", "sourceId",
               "workspaceId", "status")
           VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?6, 'connected')"#,
        params![id, now, name, channel, id.to_string(), workspace_id],
    )
    .map_err(repo_err)?;
    Ok(())
}

/// `user.disconnect` of a channel connection (reason `manual`) and the inbox
/// mirror (`mirrorInboxStatus`). An already disconnected row is unchanged.
pub(crate) fn mark_disconnected(
    conn: &Connection,
    current: &ConnectionRow,
    now: i64,
) -> Result<ConnectionRow> {
    if current.status == "disconnected" {
        return Ok(current.clone());
    }
    conn.execute(
        r#"UPDATE "Connection" SET "status" = 'disconnected', "statusReason" = 'manual',
               "disconnectedAt" = ?3, "updatedAt" = ?3
           WHERE "id" = ?1 AND "workspaceId" = ?2"#,
        params![current.id, current.workspace_id, now],
    )
    .map_err(repo_err)?;
    if let Some(inbox_id) = current.inbox_id {
        conn.execute(
            r#"UPDATE "Inbox" SET "status" = 'disconnected', "disconnectedAt" = ?3,
                   "disconnectReason" = 'manual', "updatedAt" = ?3
               WHERE "id" = ?1 AND "workspaceId" = ?2"#,
            params![inbox_id, current.workspace_id, now],
        )
        .map_err(repo_err)?;
    }
    find_or_fail(conn, current.id, current.workspace_id)
}

fn is_active(status: &str) -> bool {
    matches!(status, "connected" | "degraded" | "needs_reauth")
}

fn inactive() -> BusinessError {
    BusinessError::exception("This connection is not active.", "connectionInactive", 409)
}

/// A `Connection` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionRow {
    pub id: i64,
    pub workspace_id: i64,
    pub provider: String,
    pub kind: String,
    pub channel: Option<String>,
    pub inbox_id: Option<i64>,
    pub integration_id: Option<i64>,
    pub source_id: String,
    pub display_name: String,
    pub status: String,
    pub status_reason: Option<String>,
    pub last_error: Option<String>,
    pub auth_expires_at: Option<i64>,
    pub connected_at: Option<i64>,
    pub disconnected_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// `connectionStateService.list` input (`status` is one status here).
#[derive(Debug, Clone, Default)]
pub struct ListConnections {
    pub workspace_id: i64,
    pub kind: Option<String>,
    pub provider: Option<String>,
    pub channel: Option<String>,
    pub status: Option<String>,
    pub page: i64,
    pub per_page: i64,
}

/// `connectionStateService.list`: rows ordered by kind, provider, display
/// name, id; plus the total count.
pub async fn list(db: &Db, input: ListConnections) -> Result<(Vec<ConnectionRow>, i64)> {
    tx::read(db, move |conn| {
        let mut clauses = vec![r#""workspaceId" = ?1"#.to_string()];
        let mut binds: Vec<chatbotx_db::rusqlite::types::Value> = vec![input.workspace_id.into()];
        for (column, value) in [
            ("kind", &input.kind),
            ("provider", &input.provider),
            ("channel", &input.channel),
            ("status", &input.status),
        ] {
            if let Some(value) = value {
                binds.push(value.clone().into());
                clauses.push(format!(r#""{column}" = ?{}"#, binds.len()));
            }
        }
        let filter = clauses.join(" AND ");
        let count: i64 = conn
            .query_row(
                &format!(r#"SELECT COUNT(*) FROM "Connection" WHERE {filter}"#),
                chatbotx_db::rusqlite::params_from_iter(binds.iter()),
                |r| r.get(0),
            )
            .map_err(repo_err)?;
        let offset = (input.page.max(1) - 1) * input.per_page;
        let mut statement = conn
            .prepare(&format!(
                r#"SELECT {COLUMNS} FROM "Connection" WHERE {filter}
                   ORDER BY "kind", "provider", "displayName", "id" LIMIT {} OFFSET {offset}"#,
                input.per_page
            ))
            .map_err(repo_err)?;
        let rows = statement
            .query_map(chatbotx_db::rusqlite::params_from_iter(binds.iter()), row)
            .map_err(repo_err)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(repo_err)?;
        Ok((rows, count))
    })
    .await
}

/// `connectionStateService.getForWorkspace`.
pub async fn get_for_workspace(
    db: &Db,
    id: i64,
    workspace_id: i64,
) -> Result<Option<ConnectionRow>> {
    tx::read(db, move |conn| find(conn, id, workspace_id)).await
}

/// `connectionStateService.updateDisplayName` (the name is already trimmed).
pub async fn update_display_name(
    db: &Db,
    id: i64,
    workspace_id: i64,
    display_name: &str,
) -> Result<ConnectionRow> {
    let display_name = display_name.to_string();
    let now = repo::now_ms();
    tx::write(db, move |tx| {
        find_or_fail(tx, id, workspace_id)?;
        tx.execute(
            r#"UPDATE "Connection" SET "displayName" = ?3, "updatedAt" = ?4
               WHERE "id" = ?1 AND "workspaceId" = ?2"#,
            params![id, workspace_id, display_name, now],
        )
        .map_err(repo_err)?;
        find_or_fail(tx, id, workspace_id)
    })
    .await
}

/// `connectionService.disconnect` for a self-serve channel: `user.disconnect`
/// (status `disconnected`, reason `manual`), the inbox mirror, the satellite
/// row kept (`onDisconnect: "keep_row"`).
pub async fn disconnect(db: &Db, id: i64, workspace_id: i64) -> Result<ConnectionRow> {
    let now = repo::now_ms();
    tx::write(db, move |tx| {
        let current = find_or_fail(tx, id, workspace_id)?;
        mark_disconnected(tx, &current, now)
    })
    .await
}

/// `connectionService.verify` for a self-serve channel (`verify` is always
/// ok): `verify.ok`. An inactive connection is `connectionInactive`.
pub async fn verify(db: &Db, id: i64, workspace_id: i64) -> Result<ConnectionRow> {
    let now = repo::now_ms();
    tx::write(db, move |tx| {
        let current = find_or_fail(tx, id, workspace_id)?;
        if !is_active(&current.status) {
            return Err(inactive());
        }
        if providers::provider_meta(&current.provider).is_none() {
            return Err(not_configured(&current.provider));
        }
        if current.status == "connected" {
            return Ok(current);
        }
        tx.execute(
            r#"UPDATE "Connection" SET "status" = 'connected', "statusReason" = NULL,
                   "updatedAt" = ?3
               WHERE "id" = ?1 AND "workspaceId" = ?2"#,
            params![id, workspace_id, now],
        )
        .map_err(repo_err)?;
        find_or_fail(tx, id, workspace_id)
    })
    .await
}

fn not_configured(provider: &str) -> BusinessError {
    BusinessError::exception(
        format!("Connection provider \"{provider}\" is not configured."),
        "connectionNotConfigured",
        400,
    )
}

/// `connectionService.refresh`: inactive = `connectionInactive`; a provider
/// without `refreshAuth` = `connectionNotRefreshable`.
pub async fn refresh(db: &Db, id: i64, workspace_id: i64) -> Result<ConnectionRow> {
    let current = tx::read(db, move |conn| find_or_fail(conn, id, workspace_id)).await?;
    if !is_active(&current.status) {
        return Err(inactive());
    }
    let Some(meta) = providers::provider_meta(&current.provider) else {
        return Err(not_configured(&current.provider));
    };
    if !meta.refreshable || meta.strategy == "self_serve" || meta.strategy == "token" {
        return Err(BusinessError::exception(
            format!(
                "Connection provider \"{}\" does not support refresh.",
                current.provider
            ),
            "connectionNotRefreshable",
            400,
        ));
    }
    // The OAuth token refresh of the other providers is not ported yet.
    Err(not_configured(&current.provider))
}

/// `startReconnect` up to the OAuth part: a provider that is not OAuth (or
/// has no platform credential) is `connectionNotConfigured`.
pub async fn reconnect(_db: &Db, connection: &ConnectionRow) -> Result<ConnectionRow> {
    // `startReconnect`: a provider that is not OAuth has no OAuth credential
    // to resolve; the OAuth reconnect session is not ported yet.
    Err(not_configured(&connection.provider))
}
