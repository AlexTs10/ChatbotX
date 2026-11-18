//! `packages/business/src/connect-session/service.ts`
//! (`connectSessionService`) and its repository: the `ConnectSession` row of
//! an OAuth connect. The OAuth `state` is `"{sessionId}.{nonce}"`; only the
//! SHA-256 of the 32-byte random nonce is stored, so the callback resolves
//! the session (and its workspace, provider, actor and return URL) from the
//! nonce alone. Every guarded update runs read-check-write inside one
//! SQLite write transaction (the original: one guarded `UPDATE`).

use chatbotx_db::Db;
use chatbotx_db::repo::{self, new_id};
use chatbotx_db::rusqlite::{Connection, OptionalExtension as _, Row, params};
use serde_json::{Map, Value, json};
use sha2::{Digest as _, Sha256};

use crate::{BusinessError, Result, tx};

/// 10 min for the OAuth round trip.
pub const PENDING_TTL_MS: i64 = 10 * 60 * 1000;
/// 30 min from authorization to finish the target selection.
pub const AUTHORIZED_TTL_MS: i64 = 30 * 60 * 1000;
const MAX_PENDING_SESSIONS_PER_WORKSPACE: i64 = 20;
const TARGET_CLAIM_LEASE_MS: i64 = 5 * 60 * 1000;
const ACTIVE: &[&str] = &["pending", "authorized", "awaiting_selection"];

/// A `ConnectSession` row (never sent as is: `encryptedAuth`,
/// `stateNonceHash` and the claims stay private).
#[derive(Debug, Clone, PartialEq)]
pub struct SessionRow {
    pub id: i64,
    pub workspace_id: i64,
    pub provider: String,
    pub purpose: String,
    pub target_connection_id: Option<i64>,
    pub actor_user_id: Option<i64>,
    pub actor_token_id: Option<i64>,
    pub platform_owner_id: Option<String>,
    pub origin_host: Option<String>,
    pub return_url: Option<String>,
    pub status: String,
    pub step: String,
    pub next_action: Option<Value>,
    pub encrypted_auth: Option<String>,
    pub targets: Value,
    pub result_connection_ids: Vec<String>,
    pub error_code: Option<String>,
    pub expires_at: i64,
}

const COLUMNS: &str = r#""id", "workspaceId", "provider", "purpose", "targetConnectionId",
    "actorUserId", "actorTokenId", "platformOwnerId", "originHost", "returnUrl", "status", "step",
    "nextAction", "encryptedAuth", "targets", "resultConnectionIds", "errorCode", "expiresAt""#;

fn json_col(text: Option<String>) -> Option<Value> {
    text.and_then(|t| serde_json::from_str(&t).ok())
}

fn session(r: &Row<'_>) -> chatbotx_db::rusqlite::Result<SessionRow> {
    let ids: Vec<Value> = json_col(r.get(15)?)
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default();
    Ok(SessionRow {
        id: r.get(0)?,
        workspace_id: r.get(1)?,
        provider: r.get(2)?,
        purpose: r.get(3)?,
        target_connection_id: r.get(4)?,
        actor_user_id: r.get(5)?,
        actor_token_id: r.get(6)?,
        platform_owner_id: r.get(7)?,
        origin_host: r.get(8)?,
        return_url: r.get(9)?,
        status: r.get(10)?,
        step: r.get(11)?,
        next_action: json_col(r.get(12)?),
        encrypted_auth: r.get(13)?,
        targets: json_col(r.get(14)?).unwrap_or_else(|| json!([])),
        result_connection_ids: ids
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect(),
        error_code: r.get(16)?,
        expires_at: r.get(17)?,
    })
}

fn repo_err(error: chatbotx_db::rusqlite::Error) -> BusinessError {
    BusinessError::Repo(error.into())
}

/// `connectSessionExpiredException` (400).
pub fn expired(message: &str) -> BusinessError {
    BusinessError::exception(message, "connectSessionExpired", 400)
}

fn no_longer_active() -> BusinessError {
    expired("This connect session is no longer active.")
}

/// `ConnectSessionNotFoundException`.
fn not_found() -> BusinessError {
    BusinessError::exception("Connect session not found.", "notFound", 404)
}

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn hash_nonce(nonce: &str) -> String {
    to_hex(&Sha256::digest(nonce.as_bytes()))
}

fn load(conn: &Connection, id: i64) -> Result<Option<SessionRow>> {
    conn.query_row(
        &format!(r#"SELECT {COLUMNS} FROM "ConnectSession" WHERE "id" = ?1"#),
        [id],
        session,
    )
    .optional()
    .map_err(repo_err)
}

/// `applyExpiryRule`: an active session past `expiresAt` reads (and is
/// stored) as `expired`.
fn apply_expiry(
    conn: &Connection,
    row: Option<SessionRow>,
    now: i64,
) -> Result<Option<SessionRow>> {
    let Some(row) = row else { return Ok(None) };
    if row.expires_at > now || !ACTIVE.contains(&row.status.as_str()) {
        return Ok(Some(row));
    }
    conn.execute(
        r#"UPDATE "ConnectSession" SET "status" = 'expired', "consumedAt" = ?2,
               "encryptedAuth" = NULL, "updatedAt" = ?2
           WHERE "id" = ?1 AND "status" IN ('pending', 'authorized', 'awaiting_selection')"#,
        params![row.id, now],
    )
    .map_err(repo_err)?;
    load(conn, row.id)
}

/// The values of one guarded update ([`update_where`]).
#[derive(Default)]
struct Set {
    status: Option<&'static str>,
    step: Option<&'static str>,
    encrypted_auth: Option<Option<String>>,
    targets: Option<Value>,
    expires_at: Option<i64>,
    error_code: Option<&'static str>,
    consumed: bool,
}

/// `updateWhereStatusIn`: writes `set` only while the session is in one of
/// `statuses` (and unexpired when `unexpired`); `None` = no row matched.
fn update_where(
    conn: &Connection,
    id: i64,
    workspace_id: i64,
    statuses: &[&str],
    unexpired: bool,
    set: Set,
    now: i64,
) -> Result<Option<SessionRow>> {
    let Some(current) = load(conn, id)? else {
        return Ok(None);
    };
    if current.workspace_id != workspace_id
        || !statuses.contains(&current.status.as_str())
        || (unexpired && current.expires_at <= now)
    {
        return Ok(None);
    }
    let auth = match set.encrypted_auth {
        Some(value) => value,
        None => current.encrypted_auth.clone(),
    };
    conn.execute(
        r#"UPDATE "ConnectSession" SET "status" = ?2, "step" = ?3, "encryptedAuth" = ?4,
               "targets" = ?5, "expiresAt" = ?6, "errorCode" = ?7,
               "consumedAt" = CASE WHEN ?8 THEN ?9 ELSE "consumedAt" END, "updatedAt" = ?9
           WHERE "id" = ?1"#,
        params![
            id,
            set.status.unwrap_or(&current.status),
            set.step.unwrap_or(&current.step),
            auth,
            set.targets.unwrap_or(current.targets).to_string(),
            set.expires_at.unwrap_or(current.expires_at),
            set.error_code.or(current.error_code.as_deref()),
            set.consumed,
            now
        ],
    )
    .map_err(repo_err)?;
    load(conn, id)
}

/// The input of [`create`].
#[derive(Debug, Clone, Default)]
pub struct NewSession {
    pub workspace_id: i64,
    pub provider: String,
    pub purpose: String,
    pub target_connection_id: Option<i64>,
    pub actor_user_id: Option<i64>,
    pub actor_token_id: Option<i64>,
    pub platform_owner_id: Option<String>,
    pub origin_host: Option<String>,
    pub return_url: Option<String>,
}

fn is_safe_relative_path(value: &str) -> bool {
    value.starts_with('/')
        && !value.starts_with("//")
        && !value.contains('\\')
        && !value.chars().any(|c| (c as u32) <= 31 || c as u32 == 127)
}

/// `validateReturnUrl`: an application-relative path (normalized), else 400.
pub fn validate_return_url(value: Option<&str>) -> Result<Option<String>> {
    let Some(value) = value.filter(|v| !v.is_empty()) else {
        return Ok(None);
    };
    let refuse = || {
        BusinessError::exception(
            "Connect session return URL must be an application-relative path.",
            "validation",
            400,
        )
    };
    if !is_safe_relative_path(value) {
        return Err(refuse());
    }
    let base = url::Url::parse("http://x.invalid").map_err(|_| refuse())?;
    let url = base.join(value).map_err(|_| refuse())?;
    if url.origin() != base.origin() {
        return Err(refuse());
    }
    let mut normalized = url.path().to_string();
    if let Some(query) = url.query() {
        normalized.push('?');
        normalized.push_str(query);
    }
    if let Some(fragment) = url.fragment() {
        normalized.push('#');
        normalized.push_str(fragment);
    }
    if !is_safe_relative_path(&normalized) {
        return Err(refuse());
    }
    Ok(Some(normalized))
}

/// `create`: the per-workspace cap (20 unexpired active sessions, 429
/// `connectSessionLimitReached`), a fresh nonce, the `pending` row.
/// `next_action` builds the client action from the session id and the
/// plaintext nonce (the OAuth `state`); the nonce itself is never stored.
pub async fn create(
    db: &Db,
    input: NewSession,
    next_action: impl FnOnce(i64, &str) -> Value + Send + 'static,
) -> Result<SessionRow> {
    let return_url = validate_return_url(input.return_url.as_deref())?;
    let mut bytes = [0_u8; 32];
    ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut bytes)
        .map_err(|_| BusinessError::Repo(repo::Error::msg("random nonce")))?;
    let nonce = to_hex(&bytes);
    tx::write(db, move |conn| {
        let now = repo::now_ms();
        let active: i64 = conn
            .query_row(
                r#"SELECT count(*) FROM "ConnectSession" WHERE "workspaceId" = ?1
                   AND "status" IN ('pending', 'authorized', 'awaiting_selection') AND "expiresAt" > ?2"#,
                params![input.workspace_id, now],
                |r| r.get(0),
            )
            .map_err(repo_err)?;
        if active >= MAX_PENDING_SESSIONS_PER_WORKSPACE {
            return Err(BusinessError::exception(
                "Too many pending connect sessions for this workspace. Complete or cancel an existing one first.",
                "connectSessionLimitReached",
                429,
            ));
        }
        let id = new_id();
        let action = next_action(id, &nonce);
        conn.execute(
            r#"INSERT INTO "ConnectSession" ("id", "createdAt", "updatedAt", "workspaceId",
                   "provider", "purpose", "targetConnectionId", "actorUserId", "actorTokenId",
                   "platformOwnerId", "originHost", "returnUrl", "stateNonceHash", "status", "step",
                   "nextAction", "targets", "targetClaims", "resultConnectionIds", "results",
                   "expiresAt")
               VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, 'pending',
                   'authorize', ?13, '[]', '{}', '[]', '[]', ?14)"#,
            params![
                id,
                now,
                input.workspace_id,
                input.provider,
                input.purpose,
                input.target_connection_id,
                input.actor_user_id,
                input.actor_token_id,
                input.platform_owner_id,
                input.origin_host,
                return_url,
                hash_nonce(&nonce),
                action.to_string(),
                now + PENDING_TTL_MS
            ],
        )
        .map_err(repo_err)?;
        load(conn, id)?.ok_or_else(not_found)
    })
    .await
}

/// `findByNonce` (expiry rule applied).
pub async fn find_by_nonce(db: &Db, nonce: &str) -> Result<Option<SessionRow>> {
    let hash = hash_nonce(nonce);
    tx::write(db, move |conn| {
        let row = conn
            .query_row(
                &format!(r#"SELECT {COLUMNS} FROM "ConnectSession" WHERE "stateNonceHash" = ?1"#),
                [hash],
                session,
            )
            .optional()
            .map_err(repo_err)?;
        apply_expiry(conn, row, repo::now_ms())
    })
    .await
}

/// `findByIdForWorkspace` (expiry rule applied).
pub async fn find_for_workspace(db: &Db, id: i64, workspace_id: i64) -> Result<Option<SessionRow>> {
    tx::write(db, move |conn| {
        let row = load(conn, id)?.filter(|row| row.workspace_id == workspace_id);
        apply_expiry(conn, row, repo::now_ms())
    })
    .await
}

/// `cancel`: an active session becomes `cancelled`; any other is returned
/// unchanged.
pub async fn cancel(db: &Db, id: i64, workspace_id: i64) -> Result<SessionRow> {
    tx::write(db, move |conn| {
        let now = repo::now_ms();
        let existing = apply_expiry(
            conn,
            load(conn, id)?.filter(|row| row.workspace_id == workspace_id),
            now,
        )?
        .ok_or_else(not_found)?;
        let set = Set {
            status: Some("cancelled"),
            encrypted_auth: Some(None),
            consumed: true,
            ..Set::default()
        };
        Ok(update_where(conn, id, workspace_id, ACTIVE, false, set, now)?.unwrap_or(existing))
    })
    .await
}

/// `fail` (best effort, as `failSession`): `statuses` default to the active
/// ones; a session in another status is left as it is.
pub async fn fail(
    db: &Db,
    row: &SessionRow,
    error_code: &'static str,
    statuses: Option<&'static [&'static str]>,
) {
    let (id, ws) = (row.id, row.workspace_id);
    let result = tx::write(db, move |conn| {
        let set = Set {
            status: Some("failed"),
            error_code: Some(error_code),
            encrypted_auth: Some(None),
            consumed: true,
            ..Set::default()
        };
        update_where(
            conn,
            id,
            ws,
            statuses.unwrap_or(ACTIVE),
            false,
            set,
            repo::now_ms(),
        )
    })
    .await;
    if let Err(error) = result {
        tracing::error!(error = %error, session_id = id, error_code, "connection OAuth: failed to record terminal connect-session state");
    }
}

/// `claimAuthorization`: `pending` -> `authorized` (one callback only).
pub async fn claim_authorization(db: &Db, id: i64, workspace_id: i64) -> Result<SessionRow> {
    tx::write(db, move |conn| {
        let set = Set {
            status: Some("authorized"),
            ..Set::default()
        };
        update_where(
            conn,
            id,
            workspace_id,
            &["pending"],
            true,
            set,
            repo::now_ms(),
        )?
        .ok_or_else(no_longer_active)
    })
    .await
}

/// `releaseAuthorization`: `authorized` -> `pending` (a retryable exchange
/// failure).
pub async fn release_authorization(db: &Db, id: i64, workspace_id: i64) -> Result<()> {
    tx::write(db, move |conn| {
        let set = Set {
            status: Some("pending"),
            ..Set::default()
        };
        update_where(
            conn,
            id,
            workspace_id,
            &["authorized"],
            true,
            set,
            repo::now_ms(),
        )?;
        Ok(())
    })
    .await
}

/// `storeAuthorization`: keeps the encrypted auth of an `authorized`
/// session (30 min TTL).
pub async fn store_authorization(
    db: &Db,
    id: i64,
    workspace_id: i64,
    encrypted: String,
) -> Result<SessionRow> {
    tx::write(db, move |conn| {
        let now = repo::now_ms();
        let set = Set {
            encrypted_auth: Some(Some(encrypted)),
            expires_at: Some(now + AUTHORIZED_TTL_MS),
            ..Set::default()
        };
        update_where(conn, id, workspace_id, &["authorized"], true, set, now)?
            .ok_or_else(no_longer_active)
    })
    .await
}

/// `attachAuthorization`: the candidate list (encrypted) and the public
/// targets; `awaiting_selection` / `select` (30 min TTL).
pub async fn attach_authorization(
    db: &Db,
    id: i64,
    workspace_id: i64,
    encrypted: String,
    targets: Value,
) -> Result<SessionRow> {
    tx::write(db, move |conn| {
        let now = repo::now_ms();
        let set = Set {
            status: Some("awaiting_selection"),
            step: Some("select"),
            encrypted_auth: Some(Some(encrypted)),
            targets: Some(targets),
            expires_at: Some(now + AUTHORIZED_TTL_MS),
            ..Set::default()
        };
        update_where(conn, id, workspace_id, ACTIVE, true, set, now)?.ok_or_else(no_longer_active)
    })
    .await
}

fn claims(conn: &Connection, id: i64) -> Result<Map<String, Value>> {
    let text: String = conn
        .query_row(
            r#"SELECT "targetClaims" FROM "ConnectSession" WHERE "id" = ?1"#,
            [id],
            |r| r.get(0),
        )
        .map_err(repo_err)?;
    Ok(serde_json::from_str::<Value>(&text)
        .ok()
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default())
}

fn awaiting(conn: &Connection, id: i64, workspace_id: i64, now: i64) -> Result<bool> {
    Ok(load(conn, id)?.is_some_and(|row| {
        row.workspace_id == workspace_id
            && row.status == "awaiting_selection"
            && row.expires_at > now
    }))
}

/// `claimTarget`: a 5 min lease on one target; `false` while another
/// caller's lease runs.
pub async fn claim_target(
    db: &Db,
    id: i64,
    workspace_id: i64,
    target_id: String,
    owner: String,
) -> Result<bool> {
    tx::write(db, move |conn| {
        let now = repo::now_ms();
        if !awaiting(conn, id, workspace_id, now)? {
            return Ok(false);
        }
        let mut map = claims(conn, id)?;
        let held = map
            .get(&target_id)
            .and_then(|c| c["expiresAt"].as_i64())
            .is_some_and(|expires| expires > now);
        if held {
            return Ok(false);
        }
        map.insert(
            target_id,
            json!({ "ownerToken": owner, "expiresAt": now + TARGET_CLAIM_LEASE_MS }),
        );
        conn.execute(
            r#"UPDATE "ConnectSession" SET "targetClaims" = ?2 WHERE "id" = ?1"#,
            params![id, Value::Object(map).to_string()],
        )
        .map_err(repo_err)?;
        Ok(true)
    })
    .await
}

/// `releaseTarget`: drops this caller's own lease.
pub async fn release_target(
    db: &Db,
    id: i64,
    workspace_id: i64,
    target_id: String,
    owner: String,
) -> Result<()> {
    tx::write(db, move |conn| {
        if !awaiting(conn, id, workspace_id, repo::now_ms())? {
            return Ok(());
        }
        let mut map = claims(conn, id)?;
        if map.get(&target_id).and_then(|c| c["ownerToken"].as_str()) == Some(owner.as_str()) {
            map.remove(&target_id);
            conn.execute(
                r#"UPDATE "ConnectSession" SET "targetClaims" = ?2 WHERE "id" = ?1"#,
                params![id, Value::Object(map).to_string()],
            )
            .map_err(repo_err)?;
        }
        Ok(())
    })
    .await
}

/// `recordResults` (`appendResults`): merges the outcomes; the session is
/// `completed` once every selectable target is `connected` or
/// `duplicated`. A session no longer awaiting selection is returned as is.
pub async fn record_results(
    db: &Db,
    id: i64,
    workspace_id: i64,
    results: Vec<Value>,
    connection_ids: Vec<i64>,
) -> Result<SessionRow> {
    tx::write(db, move |conn| {
        let now = repo::now_ms();
        let current = load(conn, id)?
            .filter(|row| row.workspace_id == workspace_id)
            .ok_or_else(not_found)?;
        if current.status != "awaiting_selection" || current.expires_at <= now {
            return Ok(current);
        }
        let (old_results, old_ids): (String, String) = conn
            .query_row(
                r#"SELECT "results", "resultConnectionIds" FROM "ConnectSession" WHERE "id" = ?1"#,
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(repo_err)?;
        let mut merged: Vec<Value> = serde_json::from_str(&old_results).unwrap_or_default();
        merged.extend(results);
        let mut ids: Vec<Value> = serde_json::from_str(&old_ids).unwrap_or_default();
        ids.extend(connection_ids.iter().map(|c| json!(c.to_string())));
        let selectable: Vec<&str> = current
            .targets
            .as_array()
            .map(|t| {
                t.iter()
                    .filter(|t| t["selectable"].as_bool() == Some(true))
                    .filter_map(|t| t["id"].as_str())
                    .collect()
            })
            .unwrap_or_default();
        let resolved = selectable
            .iter()
            .filter(|target| {
                merged.iter().any(|r| {
                    r["targetId"].as_str() == Some(target)
                        && matches!(r["status"].as_str(), Some("connected" | "duplicated"))
                })
            })
            .count();
        let complete = resolved >= selectable.len();
        conn.execute(
            r#"UPDATE "ConnectSession" SET "results" = ?2, "resultConnectionIds" = ?3,
                   "status" = CASE WHEN ?4 THEN 'completed' ELSE "status" END,
                   "step" = CASE WHEN ?4 THEN 'done' ELSE "step" END,
                   "consumedAt" = CASE WHEN ?4 THEN ?5 ELSE "consumedAt" END,
                   "encryptedAuth" = CASE WHEN ?4 THEN NULL ELSE "encryptedAuth" END,
                   "updatedAt" = ?5
               WHERE "id" = ?1"#,
            params![
                id,
                Value::Array(merged).to_string(),
                Value::Array(ids).to_string(),
                complete,
                now
            ],
        )
        .map_err(repo_err)?;
        load(conn, id)?.ok_or_else(not_found)
    })
    .await
}

/// `completeReconnect` inside the caller's transaction: an `authorized`
/// reconnect session records its one result and completes.
pub fn complete_reconnect(
    conn: &Connection,
    row: &SessionRow,
    result: Value,
    connection_id: i64,
    now: i64,
) -> Result<SessionRow> {
    complete_authorized(conn, row, result, Some(connection_id), now)
}

/// The integration-connection branch (Google Calendar / Sheets): an
/// `authorized` session has no candidate step; it records its one result
/// (and the `Connection` row, when the integration has one) and completes.
pub fn complete_authorized(
    conn: &Connection,
    row: &SessionRow,
    result: Value,
    connection_id: Option<i64>,
    now: i64,
) -> Result<SessionRow> {
    let current = load(conn, row.id)?.ok_or_else(not_found)?;
    if current.status != "authorized" || current.expires_at <= now {
        return Err(no_longer_active());
    }
    let (old_results, old_ids): (String, String) = conn
        .query_row(
            r#"SELECT "results", "resultConnectionIds" FROM "ConnectSession" WHERE "id" = ?1"#,
            [row.id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(repo_err)?;
    let mut merged: Vec<Value> = serde_json::from_str(&old_results).unwrap_or_default();
    merged.push(result);
    let mut ids: Vec<Value> = serde_json::from_str(&old_ids).unwrap_or_default();
    ids.extend(connection_id.map(|id| json!(id.to_string())));
    conn.execute(
        r#"UPDATE "ConnectSession" SET "results" = ?2, "resultConnectionIds" = ?3,
               "status" = 'completed', "step" = 'done', "consumedAt" = ?4, "encryptedAuth" = NULL,
               "updatedAt" = ?4
           WHERE "id" = ?1"#,
        params![
            row.id,
            Value::Array(merged).to_string(),
            Value::Array(ids).to_string(),
            now
        ],
    )
    .map_err(repo_err)?;
    load(conn, row.id)?.ok_or_else(not_found)
}
