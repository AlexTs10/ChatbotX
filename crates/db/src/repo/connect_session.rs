//! Port of `repositories/connect-session/repository.ts`.
//!
//! JSON columns are validated on read like the zod `parseConnectSession` (an invalid
//! value is an [`Error::Coded`] of kind `ZodError` whose message names the path). The
//! jsonb updates (`jsonb_set`, `||`, `array_cat`) are done in Rust inside the one write
//! transaction, so `appendResults` stays atomic. `now` (unix ms) replaces SQL `now()`.

use rusqlite::Connection;
use rusqlite::types::Value;
use serde_json::{Map, Value as J};

use super::chunked_purge::{ChunkedPurgeOptions, ChunkedPurgeStopReason, chunked_purge};
use super::models::ConnectSession;
use super::sql::Sql;
use super::{Error, Json, Model, Result, iso, new_id, parse_iso};
use crate::Db;

/// `ACTIVE_CONNECT_SESSION_STATUSES`.
pub const ACTIVE_CONNECT_SESSION_STATUSES: &[&str] = &["pending", "authorized", "awaiting_selection"];

fn zod(path: &str, message: &str) -> Error {
    Error::Coded {
        kind: "ZodError",
        code: "invalid".to_owned(),
        message: format!("{message} at path [\"{path}\"]"),
    }
}

fn check_next_action(v: &J) -> Result<()> {
    let Some(obj) = v.as_object() else {
        return Err(zod("type", "Invalid input: expected object"));
    };
    match obj.get("type").and_then(J::as_str) {
        Some("open_url") if obj.get("url").is_some_and(J::is_string) => Ok(()),
        Some("open_url") => Err(zod("url", "Invalid input: expected string")),
        Some("enter_input") if obj.get("inputFields").is_some_and(J::is_array) => Ok(()),
        Some("enter_input") => Err(zod("inputFields", "Invalid input: expected array")),
        Some("wait") => Ok(()),
        _ => Err(zod("type", "Invalid discriminator value")),
    }
}

fn check_targets(v: &J) -> Result<()> {
    let Some(items) = v.as_array() else {
        return Err(zod("targets", "Invalid input: expected array"));
    };
    for t in items {
        let ok = t.get("id").is_some_and(J::is_string)
            && t.get("name").is_some_and(J::is_string)
            && t.get("selectable").is_some_and(J::is_boolean);
        if !ok {
            return Err(zod("targets", "Invalid target"));
        }
    }
    Ok(())
}

fn check_claims(v: &J) -> Result<()> {
    let Some(map) = v.as_object() else {
        return Err(zod("targetClaims", "Invalid input: expected record"));
    };
    for claim in map.values() {
        let owner_ok = claim.get("ownerToken").and_then(J::as_str).is_some_and(|s| !s.is_empty());
        let at_ok = claim.get("expiresAt").and_then(J::as_str).is_some_and(|s| parse_iso(s).is_some());
        if !(owner_ok && at_ok) {
            return Err(zod("targetClaims", "Invalid target claim"));
        }
    }
    Ok(())
}

const OUTCOME_STATUSES: &[&str] = &["connected", "duplicated", "limitReached", "failed"];

fn check_results(v: &J) -> Result<()> {
    let Some(items) = v.as_array() else {
        return Err(zod("results", "Invalid input: expected array"));
    };
    for r in items {
        let ok = r.get("targetId").is_some_and(J::is_string)
            && r.get("status").and_then(J::as_str).is_some_and(|s| OUTCOME_STATUSES.contains(&s));
        if !ok {
            return Err(zod("results", "Invalid outcome"));
        }
    }
    Ok(())
}

/// `parseConnectSession`: validates the JSON columns of a stored session.
pub fn parse_connect_session(session: ConnectSession) -> Result<ConnectSession> {
    todo!("P3.2");
    if let Some(next) = &session.next_action {
        check_next_action(next)?;
    }
    check_targets(&session.targets)?;
    check_claims(&session.target_claims)?;
    check_results(&session.results)?;
    Ok(session)
}

fn parsed(row: Option<ConnectSession>) -> Result<Option<ConnectSession>> {
    row.map(parse_connect_session).transpose()
}

/// `findByIdForWorkspace`.
pub fn find_by_id_for_workspace(conn: &Connection, id: i64, workspace_id: i64) -> Result<Option<ConnectSession>> {
    todo!("P3.2");
    parsed(super::find_one(conn, "\"id\" = ?1 AND \"workspaceId\" = ?2", &[&id, &workspace_id])?)
}

/// `findById`.
pub fn find_by_id(conn: &Connection, id: i64) -> Result<Option<ConnectSession>> {
    todo!("P3.2");
    parsed(super::find_one(conn, "\"id\" = ?1", &[&id])?)
}

/// `findByStateNonceHash`.
pub fn find_by_state_nonce_hash(conn: &Connection, state_nonce_hash: &str) -> Result<Option<ConnectSession>> {
    todo!("P3.2");
    parsed(super::find_one(conn, "\"stateNonceHash\" = ?1", &[&state_nonce_hash])?)
}

/// `findLatestInFlightByProvider`: the newest active, unexpired session of the provider.
pub fn find_latest_in_flight_by_provider(
    conn: &Connection,
    workspace_id: i64,
    provider: &str,
    now: i64,
) -> Result<Option<ConnectSession>> {
    parsed(super::find_one(
        conn,
        "\"workspaceId\" = ?1 AND \"provider\" = ?2 AND \"status\" IN ('pending', 'authorized', 'awaiting_selection') \
         AND \"expiresAt\" > ?3 ORDER BY \"createdAt\" DESC",
        &[&workspace_id, &provider, &now],
    )?)
}

/// `countActiveByWorkspaceId`: active, unexpired sessions of the workspace.
pub fn count_active_by_workspace_id(conn: &Connection, workspace_id: i64, now: i64) -> Result<i64> {
    todo!("P3.2");
    Ok(conn.query_row(
        "SELECT count(*) FROM \"ConnectSession\" WHERE \"workspaceId\" = ?1 \
         AND \"status\" IN ('pending', 'authorized', 'awaiting_selection') AND \"expiresAt\" > ?2",
        [workspace_id, now],
        |row| row.get(0),
    )?)
}

/// Insert values of [`insert`] (the columns without a DB default are required).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ConnectSessionInsert {
    pub workspace_id: i64,
    pub provider: String,
    pub purpose: String,
    pub target_connection_id: Option<i64>,
    pub actor_user_id: Option<i64>,
    pub actor_token_id: Option<i64>,
    pub platform_owner_id: Option<String>,
    pub origin_host: Option<String>,
    pub return_url: Option<String>,
    pub state_nonce_hash: String,
    pub status: Option<String>,
    pub step: Option<String>,
    pub next_action: Option<Json>,
    pub encrypted_auth: Option<Json>,
    pub targets: Option<Json>,
    pub expires_at: i64,
}

/// `insert`.
pub fn insert(conn: &Connection, v: &ConnectSessionInsert) -> Result<ConnectSession> {
    let now = super::now_ms();
    let rows = super::returning::<ConnectSession>(
        conn,
        "INSERT INTO \"ConnectSession\" (\"id\", \"createdAt\", \"updatedAt\", \"workspaceId\", \"provider\", \"purpose\", \
         \"targetConnectionId\", \"actorUserId\", \"actorTokenId\", \"platformOwnerId\", \"originHost\", \"returnUrl\", \
         \"stateNonceHash\", \"status\", \"step\", \"nextAction\", \"encryptedAuth\", \"targets\", \"expiresAt\") \
         VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, COALESCE(?13, 'pending'), COALESCE(?14, 'authorize'), \
         ?15, ?16, COALESCE(?17, '[]'), ?18)",
        &[
            &new_id(),
            &now,
            &v.workspace_id,
            &v.provider,
            &v.purpose,
            &v.target_connection_id,
            &v.actor_user_id,
            &v.actor_token_id,
            &v.platform_owner_id,
            &v.origin_host,
            &v.return_url,
            &v.state_nonce_hash,
            &v.status,
            &v.step,
            &v.next_action,
            &v.encrypted_auth,
            &v.targets,
            &v.expires_at,
        ],
    )?;
    let row = rows.into_iter().next().ok_or_else(|| Error::msg("insert returned no row"))?;
    parse_connect_session(row)
}

/// The `values` of [`update_where_status_in`]: `None` = untouched; `Some(None)` = NULL.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ConnectSessionPatch {
    pub encrypted_auth: Option<Option<Json>>,
    pub expires_at: Option<i64>,
    pub next_action: Option<Option<Json>>,
    pub return_url: Option<Option<String>>,
    pub status: Option<String>,
    pub step: Option<String>,
    pub targets: Option<Json>,
    pub error_code: Option<Option<String>>,
    pub consumed_at: Option<Option<i64>>,
}

/// `updateWhereStatusIn`: updates the session of the workspace only while its status is
/// in `statuses` (and, with `require_unexpired`, it is not expired).
pub fn update_where_status_in(
    conn: &Connection,
    id: i64,
    workspace_id: i64,
    values: &ConnectSessionPatch,
    statuses: &[&str],
    require_unexpired: bool,
    now: i64,
) -> Result<Option<ConnectSession>> {
    todo!("P3.2");
    let mut q = Sql::bind("UPDATE \"ConnectSession\" SET \"updatedAt\" = ", now);
    let json_opt = |v: &Option<Json>| v.as_ref().map_or(Value::Null, |j| Value::Text(j.0.to_string()));
    if let Some(v) = &values.encrypted_auth {
        q.push(", \"encryptedAuth\" = ");
        q.push_bind(json_opt(v));
    }
    if let Some(v) = values.expires_at {
        q.push(", \"expiresAt\" = ");
        q.push_bind(v);
    }
    if let Some(v) = &values.next_action {
        q.push(", \"nextAction\" = ");
        q.push_bind(json_opt(v));
    }
    if let Some(v) = &values.return_url {
        q.push(", \"returnUrl\" = ");
        q.push_bind(v.clone().map_or(Value::Null, Value::Text));
    }
    if let Some(v) = &values.status {
        q.push(", \"status\" = ");
        q.push_bind(v.clone());
    }
    if let Some(v) = &values.step {
        q.push(", \"step\" = ");
        q.push_bind(v.clone());
    }
    if let Some(v) = &values.targets {
        q.push(", \"targets\" = ");
        q.push_bind(v.0.to_string());
    }
    if let Some(v) = &values.error_code {
        q.push(", \"errorCode\" = ");
        q.push_bind(v.clone().map_or(Value::Null, Value::Text));
    }
    if let Some(v) = values.consumed_at {
        q.push(", \"consumedAt\" = ");
        q.push_bind(v.map_or(Value::Null, Value::Integer));
    }
    q.push(" WHERE \"id\" = ");
    q.push_bind(id);
    q.push(" AND \"workspaceId\" = ");
    q.push_bind(workspace_id);
    q.push(" AND \"status\" IN (SELECT value FROM json_each(");
    q.push_bind(super::text_list(statuses));
    q.push("))");
    if require_unexpired {
        q.push(" AND \"expiresAt\" > ");
        q.push_bind(now);
    }
    q.push(&format!(" RETURNING {}", ConnectSession::columns_sql(None)));
    parsed(q.model(conn)?)
}

/// `cancelPendingByProvider`: cancels (consumed, ciphertext cleared) the pending sessions
/// of one workspace and provider; returns how many.
pub fn cancel_pending_by_provider(conn: &Connection, workspace_id: i64, provider: &str, now: i64) -> Result<usize> {
    todo!("P3.2");
    Ok(conn.execute(
        "UPDATE \"ConnectSession\" SET \"status\" = 'cancelled', \"consumedAt\" = ?3, \"encryptedAuth\" = NULL, \"updatedAt\" = ?3 \
         WHERE \"workspaceId\" = ?1 AND \"provider\" = ?2 AND \"status\" = 'pending'",
        rusqlite::params![workspace_id, provider, now],
    )?)
}

/// `expireDue`: sessions in `statuses` whose `expiresAt <= before` become `expired`.
pub fn expire_due(conn: &Connection, before: i64, statuses: &[&str], now: i64) -> Result<usize> {
    todo!("P3.2");
    Ok(conn.execute(
        "UPDATE \"ConnectSession\" SET \"status\" = 'expired', \"consumedAt\" = ?3, \"encryptedAuth\" = NULL, \"updatedAt\" = ?3 \
         WHERE \"status\" IN (SELECT value FROM json_each(?1)) AND \"expiresAt\" <= ?2",
        rusqlite::params![super::text_list(statuses), before, now],
    )?)
}

/// `purgeOldTerminal`: chunked delete of sessions consumed more than `retention_days` ago.
pub async fn purge_old_terminal(
    db: &Db,
    retention_days: i64,
    chunk_size: i64,
    inter_chunk_delay_ms: u64,
    max_chunks: i64,
    max_run_duration_ms: Option<i64>,
) -> Result<(i64, ChunkedPurgeStopReason)> {
    todo!("P3.2");
    let cutoff = super::now_ms() - retention_days * 86_400_000;
    chunked_purge(
        db,
        &ChunkedPurgeOptions {
            table: "ConnectSession".into(),
            filter: Sql::bind("\"consumedAt\" IS NOT NULL AND \"consumedAt\" < ", cutoff),
            order_by: "consumedAt".into(),
            chunk_size,
            inter_chunk_delay_ms,
            max_chunks,
            max_run_duration_ms,
        },
    )
    .await
}

/// The JSON path of one key of `targetClaims` (`$."<key>"`).
fn claim_path(target_id: &str) -> String {
    format!("$.\"{}\"", target_id.replace('\\', "\\\\").replace('"', "\\\""))
}

/// `claimTarget`: takes the target's lease when it has none or an expired one (session of
/// the workspace, awaiting selection, unexpired).
pub fn claim_target(
    conn: &Connection,
    id: i64,
    workspace_id: i64,
    target_id: &str,
    owner_token: &str,
    lease_expires_at: i64,
    now: i64,
) -> Result<bool> {
    todo!("P3.2");
    let path = claim_path(target_id);
    let n = conn.execute(
        "UPDATE \"ConnectSession\" SET \"targetClaims\" = json_set(\"targetClaims\", ?1, json_object('ownerToken', ?2, 'expiresAt', ?3)) \
         WHERE \"id\" = ?4 AND \"workspaceId\" = ?5 AND \"status\" = 'awaiting_selection' AND \"expiresAt\" > ?6 \
           AND (json_extract(\"targetClaims\", ?1) IS NULL \
             OR CAST(unixepoch(json_extract(\"targetClaims\", ?1 || '.expiresAt'), 'subsec') * 1000 AS INTEGER) <= ?6)",
        rusqlite::params![path, owner_token, iso(lease_expires_at), id, workspace_id, now],
    )?;
    Ok(n > 0)
}

/// `releaseTarget`: drops the claim only while `owner_token` still holds it (same guards
/// as [`claim_target`]).
pub fn release_target(
    conn: &Connection,
    id: i64,
    workspace_id: i64,
    target_id: &str,
    owner_token: &str,
    now: i64,
) -> Result<()> {
    todo!("P3.2");
    let path = claim_path(target_id);
    conn.execute(
        "UPDATE \"ConnectSession\" SET \"targetClaims\" = json_remove(\"targetClaims\", ?1) \
         WHERE \"id\" = ?2 AND \"workspaceId\" = ?3 AND \"status\" = 'awaiting_selection' AND \"expiresAt\" > ?4 \
           AND json_extract(\"targetClaims\", ?1 || '.ownerToken') = ?5",
        rusqlite::params![path, id, workspace_id, now, owner_token],
    )?;
    Ok(())
}

fn append_ids(current: &J, ids: &[String]) -> J {
    let mut list = current.as_array().cloned().unwrap_or_default();
    list.extend(ids.iter().map(|s| J::from(s.clone())));
    J::Array(list)
}

fn guarded_session(conn: &Connection, id: i64, workspace_id: i64, status: &str, now: i64) -> Result<Option<ConnectSession>> {
    super::find_one(
        conn,
        "\"id\" = ?1 AND \"workspaceId\" = ?2 AND \"status\" = ?3 AND \"expiresAt\" > ?4",
        &[&id, &workspace_id, &status, &now],
    )
}

/// `appendResults`: appends outcomes (and connection ids) to an awaiting-selection,
/// unexpired session of the workspace. When every selectable target has a `connected` or
/// `duplicated` outcome the session completes (`done`, consumed, ciphertext cleared).
/// Outcomes of unknown or non-selectable targets never count. Run inside one write.
pub fn append_results(
    conn: &Connection,
    id: i64,
    workspace_id: i64,
    results: &[J],
    result_connection_ids: &[String],
    now: i64,
) -> Result<Option<ConnectSession>> {
    todo!("P3.2");
    let Some(session) = guarded_session(conn, id, workspace_id, "awaiting_selection", now)? else {
        return Ok(None);
    };
    let mut merged = session.results.0.as_array().cloned().unwrap_or_default();
    merged.extend(results.iter().cloned());
    let selectable: Vec<String> = session
        .targets
        .0
        .as_array()
        .map(|ts| {
            ts.iter()
                .filter(|t| t.get("selectable").and_then(J::as_bool) == Some(true))
                .filter_map(|t| t.get("id").and_then(J::as_str).map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    let resolved: std::collections::HashSet<&str> = merged
        .iter()
        .filter(|r| matches!(r.get("status").and_then(J::as_str), Some("connected" | "duplicated")))
        .filter_map(|r| r.get("targetId").and_then(J::as_str))
        .filter(|t| selectable.iter().any(|s| s == t))
        .collect();
    let complete = resolved.len() >= selectable.len();
    let ids = append_ids(&session.result_connection_ids.0, result_connection_ids);
    let sql = format!(
        "UPDATE \"ConnectSession\" SET \"results\" = ?1, \"resultConnectionIds\" = ?2, \"updatedAt\" = ?3{} WHERE \"id\" = ?4 RETURNING {}",
        if complete {
            ", \"status\" = 'completed', \"step\" = 'done', \"consumedAt\" = ?3, \"encryptedAuth\" = NULL"
        } else {
            ""
        },
        ConnectSession::columns_sql(None)
    );
    let rows = super::query_models::<ConnectSession>(conn, &sql, &[&J::Array(merged).to_string(), &ids.to_string(), &now, &id])?;
    parsed(rows.into_iter().next())
}

/// `completeReconnect`: completes an authorized, unexpired session of the workspace with
/// one outcome (`result` must carry a `connectionId`).
pub fn complete_reconnect(
    conn: &Connection,
    id: i64,
    workspace_id: i64,
    result: &J,
    now: i64,
) -> Result<Option<ConnectSession>> {
    todo!("P3.2");
    let Some(session) = guarded_session(conn, id, workspace_id, "authorized", now)? else {
        return Ok(None);
    };
    let mut merged = session.results.0.as_array().cloned().unwrap_or_default();
    merged.push(result.clone());
    let connection_id = result.get("connectionId").and_then(J::as_str).unwrap_or_default().to_owned();
    let ids = append_ids(&session.result_connection_ids.0, &[connection_id]);
    let sql = format!(
        "UPDATE \"ConnectSession\" SET \"results\" = ?1, \"resultConnectionIds\" = ?2, \"status\" = 'completed', \"step\" = 'done', \
         \"consumedAt\" = ?3, \"encryptedAuth\" = NULL, \"updatedAt\" = ?3 WHERE \"id\" = ?4 RETURNING {}",
        ConnectSession::columns_sql(None)
    );
    let rows = super::query_models::<ConnectSession>(conn, &sql, &[&J::Array(merged).to_string(), &ids.to_string(), &now, &id])?;
    parsed(rows.into_iter().next())
}

/// An empty JSON object (default of `targetClaims`).
pub fn empty_claims() -> Json {
    Json(J::Object(Map::new()))
}
