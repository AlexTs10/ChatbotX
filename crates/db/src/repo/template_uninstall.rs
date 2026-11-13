//! Port of `repositories/template-uninstall/` (`templateUninstallStateRepository`: claims,
//! tombstones, effects, operations, install roots; and
//! `templateUninstallReferenceRepository`); no TS tests, smoke test
//! `tests/repo_p33_smoke.rs`.
//!
//! - `now()` / `new Date()` = `now_ms()`; the lease bound `now() - 300 s` =
//!   `now_ms() - TEMPLATE_UNINSTALL_LEASE_SECONDS * 1000`.
//! - Row locks (`FOR UPDATE`) are plain reads under the one SQLite writer; run the
//!   multi-statement functions in one write job.
//! - Every UPDATE of a table with `"updatedAt"` sets it (Drizzle `$onUpdate`).

use std::collections::BTreeSet;

use rusqlite::Connection;
use rusqlite::types::Value;

use super::models::TemplateUninstallOperation;
use super::patch;
use super::sql::{Sql, opt_int, opt_text};
use super::template::{TEMPLATE_UNINSTALL_COUNT_KEYS, parse_template_uninstall_counts};
use super::template_installed_resource_uninstall::TEMPLATE_UNINSTALL_DOOMED;
use super::{Error, Model, Result, new_id, now_ms, text_list};

/// `TEMPLATE_UNINSTALL_LEASE_SECONDS`.
pub const TEMPLATE_UNINSTALL_LEASE_SECONDS: i64 = 300;

/// `FINAL_STATES`.
pub const FINAL_STATES: [&str; 4] = ["done", "partial", "failed", "skipped"];

const REQUESTABLE_STATES: [&str; 4] = ["none", "partial", "failed", "skipped"];

const TI: &str = "\"TemplateInstallation\"";

fn lease_expired() -> i64 {
    now_ms() - TEMPLATE_UNINSTALL_LEASE_SECONDS * 1000
}

/// `emptyTemplateUninstallCounts`.
pub fn empty_counts() -> serde_json::Value {
    let mut out = serde_json::Map::new();
    for key in TEMPLATE_UNINSTALL_COUNT_KEYS {
        out.insert(key.to_owned(), serde_json::Value::from(0));
    }
    serde_json::Value::Object(out)
}

/// `sumTemplateUninstallCounts` (both sides parsed counts).
pub fn sum_counts(a: &serde_json::Value, b: &serde_json::Value) -> serde_json::Value {
    let mut out = serde_json::Map::new();
    for key in TEMPLATE_UNINSTALL_COUNT_KEYS {
        let n = a.get(key).and_then(serde_json::Value::as_i64).unwrap_or(0)
            + b.get(key).and_then(serde_json::Value::as_i64).unwrap_or(0);
        out.insert(key.to_owned(), serde_json::Value::from(n));
    }
    serde_json::Value::Object(out)
}

/// An UPDATE of `TemplateInstallation ti` (plus `updatedAt`) where `cond`; the count.
fn update_ti(conn: &Connection, set: Sql, cond: Sql) -> Result<usize> {
    update_ti_sql(set, cond).execute(conn)
}

fn update_ti_sql(set: Sql, cond: Sql) -> Sql {
    let mut sql = Sql::raw(format!("UPDATE {TI} AS ti SET "));
    sql.append(set);
    sql.push(", \"updatedAt\" = ");
    sql.push_bind(now_ms());
    sql.push(" WHERE ");
    sql.append(cond);
    sql
}

// ---------------------------------------------------------------------------
// operations.ts
// ---------------------------------------------------------------------------

/// `worstState`.
fn worst_state(states: &BTreeSet<String>) -> &'static str {
    for state in ["failed", "partial"] {
        if states.contains(state) {
            return state;
        }
    }
    if states.contains("skipped") && !states.contains("done") {
        "skipped"
    } else {
        "done"
    }
}

/// `createOperation`.
pub fn create_operation(
    conn: &Connection,
    workspace_id: i64,
    template_id: Option<i64>,
    publisher_ref: &str,
    requested_by_kind: &str,
    requested_by: Option<i64>,
) -> Result<TemplateUninstallOperation> {
    Sql::new(
        format!(
            "INSERT INTO \"TemplateUninstallOperation\" (\"id\", \"workspaceId\", \"templateId\", \"publisherRef\", \"requestedByKind\", \"requestedBy\", \"auditLogId\") VALUES (?, ?, ?, ?, ?, ?, ?) RETURNING {}",
            TemplateUninstallOperation::columns_sql(None)
        ),
        vec![
            Value::Integer(new_id()),
            Value::Integer(workspace_id),
            opt_int(template_id),
            Value::Text(publisher_ref.to_owned()),
            Value::Text(requested_by_kind.to_owned()),
            opt_int(requested_by),
            Value::Text(new_id().to_string()),
        ],
    )
    .model(conn)?
    .ok_or_else(|| Error::msg("uninstall operation insert returned no row"))
}

/// `operationIdOf`.
pub fn operation_id_of(conn: &Connection, installation_id: i64) -> Result<Option<i64>> {
    Ok(Sql::bind(
        format!("SELECT \"uninstallOperationId\" FROM {TI} WHERE \"id\" = "),
        installation_id,
    )
    .query_map(conn, |row| row.get::<_, Option<i64>>(0))?
    .into_iter()
    .next()
    .flatten())
}

/// `operationHasActiveMembers`.
pub fn operation_has_active_members(conn: &Connection, operation_id: i64) -> Result<bool> {
    Ok(!Sql::bind(
        format!(
            "SELECT 1 FROM {TI} WHERE \"uninstallState\" IN ('pending', 'running') AND \"uninstallOperationId\" = "
        ),
        operation_id,
    )
    .closed(" LIMIT 1")
    .query_map(conn, |row| row.get::<_, i64>(0))?
    .is_empty())
}

/// `finalizeOperationIfComplete`: sums the members and closes a pending operation with no
/// active member. Run it in one write job.
pub fn finalize_operation_if_complete(conn: &Connection, operation_id: i64) -> Result<bool> {
    let exists = !Sql::bind(
        "SELECT 1 FROM \"TemplateUninstallOperation\" WHERE \"id\" = ",
        operation_id,
    )
    .query_map(conn, |row| row.get::<_, i64>(0))?
    .is_empty();
    if !exists || operation_has_active_members(conn, operation_id)? {
        return Ok(false);
    }
    let members: Vec<(String, Option<String>)> = Sql::bind(
        format!("SELECT \"uninstallState\", \"uninstallCounts\" FROM {TI} WHERE \"uninstallOperationId\" = "),
        operation_id,
    )
    .query_map(conn, |row| Ok((row.get(0)?, row.get(1)?)))?;
    let mut counts = empty_counts();
    for (_, raw) in &members {
        let parsed = raw
            .as_deref()
            .and_then(|t| serde_json::from_str::<serde_json::Value>(t).ok())
            .and_then(|v| parse_template_uninstall_counts(&v))
            .unwrap_or_else(empty_counts);
        counts = sum_counts(&counts, &parsed);
    }
    let states: BTreeSet<String> = members.into_iter().map(|(s, _)| s).collect();
    let state = if states.is_empty() {
        "skipped"
    } else {
        worst_state(&states)
    };
    let now = now_ms();
    let n = conn.execute(
        "UPDATE \"TemplateUninstallOperation\" SET \"state\" = ?1, \"counts\" = ?2, \"finishedAt\" = ?3, \"updatedAt\" = ?3 WHERE \"id\" = ?4 AND \"state\" = 'pending'",
        (state, counts.to_string(), now, operation_id),
    )?;
    Ok(n > 0)
}

/// `listOperationsToFinalize`.
pub fn list_operations_to_finalize(conn: &Connection, limit: i64) -> Result<Vec<i64>> {
    Sql::bind(
        format!(
            "SELECT o.\"id\" FROM \"TemplateUninstallOperation\" o WHERE o.\"state\" = 'pending' AND NOT EXISTS (SELECT 1 FROM {TI} i WHERE i.\"uninstallOperationId\" = o.\"id\" AND i.\"uninstallState\" IN ('pending', 'running')) LIMIT "
        ),
        limit,
    )
    .query_map(conn, |row| row.get(0))
}

/// `listUndeliveredNotices`.
pub fn list_undelivered_notices(
    conn: &Connection,
    limit: i64,
) -> Result<Vec<TemplateUninstallOperation>> {
    Sql::new(
        format!(
            "{} WHERE \"state\" IN (SELECT value FROM json_each(?)) AND \"noticeDeliveredAt\" IS NULL LIMIT ?",
            TemplateUninstallOperation::select_sql()
        ),
        vec![Value::Text(text_list(&FINAL_STATES)), Value::Integer(limit)],
    )
    .models(conn)
}

/// `markNoticeDelivered`.
pub fn mark_notice_delivered(conn: &Connection, operation_id: i64) -> Result<()> {
    let now = now_ms();
    conn.execute(
        "UPDATE \"TemplateUninstallOperation\" SET \"noticeDeliveredAt\" = ?1, \"updatedAt\" = ?1 WHERE \"id\" = ?2",
        (now, operation_id),
    )?;
    Ok(())
}

/// `findOperation`.
pub fn find_operation(
    conn: &Connection,
    id: i64,
    workspace_id: i64,
) -> Result<Option<TemplateUninstallOperation>> {
    Sql::new(
        format!(
            "{} WHERE \"id\" = ? AND \"workspaceId\" = ?",
            TemplateUninstallOperation::select_sql()
        ),
        vec![Value::Integer(id), Value::Integer(workspace_id)],
    )
    .model(conn)
}

// ---------------------------------------------------------------------------
// claims.ts
// ---------------------------------------------------------------------------

/// `heldBy(claim)`.
fn held_by(id: i64, token: &str) -> Sql {
    Sql::new(
        "ti.\"id\" = ? AND ti.\"uninstallClaimToken\" = ? AND ti.\"uninstallState\" = 'running'",
        vec![Value::Integer(id), Value::Text(token.to_owned())],
    )
}

/// `pending OR (running AND lease expired)`.
fn claimable() -> Sql {
    Sql::bind(
        "(ti.\"uninstallState\" = 'pending' OR (ti.\"uninstallState\" = 'running' AND ti.\"uninstallClaimedAt\" < ",
        lease_expired(),
    )
    .closed("))")
}

/// `request`: an active install gets an uninstall request; `(attempt, workspaceId)`.
pub fn request(
    conn: &Connection,
    id: i64,
    operation_id: i64,
    requested_by_kind: &str,
    requested_by: Option<i64>,
) -> Result<Option<(i64, i64)>> {
    let mut sql = update_ti_sql(
        Sql::new(
            "\"uninstallState\" = 'pending', \"uninstallAttempt\" = \"uninstallAttempt\" + 1, \"uninstallOperationId\" = ?, \"uninstallRequestedByKind\" = ?, \"uninstallRequestedBy\" = ?, \"uninstallRequestedAt\" = ?, \"uninstallClaimToken\" = NULL, \"uninstallClaimedAt\" = NULL, \"uninstallErrorCode\" = NULL, \"autoUpdate\" = 0",
            vec![
                Value::Integer(operation_id),
                Value::Text(requested_by_kind.to_owned()),
                opt_int(requested_by),
                Value::Integer(now_ms()),
            ],
        ),
        Sql::new(
            "ti.\"id\" = ? AND ti.\"status\" IN ('completed', 'partial') AND ti.\"uninstallState\" IN (SELECT value FROM json_each(?))",
            vec![
                Value::Integer(id),
                Value::Text(text_list(&REQUESTABLE_STATES)),
            ],
        ),
    );
    sql.push(" RETURNING \"uninstallAttempt\", \"workspaceId\"");
    Ok(sql
        .query_map(conn, |row| Ok((row.get(0)?, row.get(1)?)))?
        .into_iter()
        .next())
}

/// `claim`: a pending (or lease-expired running) uninstall becomes `running`;
/// `(id, workspaceId)`.
pub fn claim(
    conn: &Connection,
    id: i64,
    workspace_id: i64,
    token: &str,
) -> Result<Option<(i64, i64)>> {
    let mut cond = Sql::new(
        "ti.\"id\" = ? AND ti.\"workspaceId\" = ? AND ",
        vec![Value::Integer(id), Value::Integer(workspace_id)],
    );
    cond.append(claimable());
    let mut sql = update_ti_sql(
        Sql::new(
            "\"uninstallState\" = 'running', \"uninstallClaimToken\" = ?, \"uninstallClaimedAt\" = ?",
            vec![Value::Text(token.to_owned()), Value::Integer(now_ms())],
        ),
        cond,
    );
    sql.push(" RETURNING \"id\", \"workspaceId\"");
    Ok(sql
        .query_map(conn, |row| Ok((row.get(0)?, row.get(1)?)))?
        .into_iter()
        .next())
}

/// `hasLiveUninstallTouchingWorkspace`: a running, leased uninstall of or from the
/// workspace.
pub fn has_live_uninstall_touching_workspace(conn: &Connection, workspace_id: i64) -> Result<bool> {
    Ok(Sql::new(
        format!(
            "SELECT EXISTS (SELECT 1 FROM {TI} i WHERE i.\"uninstallState\" = 'running' AND i.\"uninstallClaimedAt\" >= ? AND (i.\"workspaceId\" = ? OR i.\"templateId\" IN (SELECT t.\"id\" FROM \"Template\" t WHERE t.\"workspaceId\" = ?)))"
        ),
        vec![
            Value::Integer(lease_expired()),
            Value::Integer(workspace_id),
            Value::Integer(workspace_id),
        ],
    )
    .query_map(conn, |row| row.get::<_, bool>(0))?
    .into_iter()
    .next()
    .unwrap_or(false))
}

/// `renewLease`.
pub fn renew_lease(conn: &Connection, id: i64, token: &str) -> Result<bool> {
    Ok(update_ti(
        conn,
        Sql::bind("\"uninstallClaimedAt\" = ", now_ms()),
        held_by(id, token),
    )? > 0)
}

/// `release`.
pub fn release(conn: &Connection, id: i64, token: &str) -> Result<()> {
    update_ti(
        conn,
        Sql::raw("\"uninstallState\" = 'pending', \"uninstallClaimToken\" = NULL"),
        held_by(id, token),
    )?;
    Ok(())
}

/// `finalize`: the claim holder records the final state (`done` also uninstalls).
pub fn finalize(
    conn: &Connection,
    id: i64,
    token: &str,
    state: &str,
    counts: &serde_json::Value,
    error_code: Option<&str>,
) -> Result<bool> {
    let mut set = Sql::new(
        "\"uninstallState\" = ?, \"uninstallClaimToken\" = NULL, \"uninstallErrorCode\" = ?, \"uninstallCounts\" = ?, \"autoUpdate\" = 0",
        vec![
            Value::Text(state.to_owned()),
            opt_text(error_code),
            patch::json(counts),
        ],
    );
    if state == "done" {
        set.push(", \"status\" = 'uninstalled', \"uninstalledByKind\" = \"uninstallRequestedByKind\", \"uninstalledAt\" = ");
        set.push_bind(now_ms());
    }
    Ok(update_ti(conn, set, held_by(id, token))? > 0)
}

/// `markSkipped`.
pub fn mark_skipped(conn: &Connection, id: i64, error_code: &str) -> Result<bool> {
    Ok(update_ti(
        conn,
        Sql::bind(
            "\"uninstallState\" = 'skipped', \"uninstallClaimToken\" = NULL, \"uninstallErrorCode\" = ",
            error_code.to_owned(),
        ),
        Sql::bind(
            "ti.\"uninstallState\" IN ('pending', 'running') AND ti.\"id\" = ",
            id,
        ),
    )? > 0)
}

/// `listStale`: `(id, workspaceId, attempt)`.
pub fn list_stale(conn: &Connection, limit: i64) -> Result<Vec<(i64, i64, i64)>> {
    let mut sql = Sql::raw(format!(
        "SELECT ti.\"id\", ti.\"workspaceId\", ti.\"uninstallAttempt\" FROM {TI} ti WHERE "
    ));
    sql.append(claimable());
    sql.push(" LIMIT ");
    sql.push_bind(limit);
    sql.query_map(conn, |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
}

/// `listPendingPublisherUninstalls`: `(id, operationId)`.
pub fn list_pending_publisher_uninstalls(
    conn: &Connection,
    workspace_id: i64,
    source_template_id: i64,
) -> Result<Vec<(i64, Option<i64>)>> {
    Sql::new(
        format!(
            "SELECT \"id\", \"uninstallOperationId\" FROM {TI} WHERE \"workspaceId\" = ? AND \"sourceTemplateId\" = ? AND \"uninstallState\" = 'pending' AND \"uninstallRequestedByKind\" = 'publisher'"
        ),
        vec![Value::Integer(workspace_id), Value::Integer(source_template_id)],
    )
    .query_map(conn, |row| Ok((row.get(0)?, row.get(1)?)))
}

/// `skipPending`.
pub fn skip_pending(conn: &Connection, id: i64, error_code: &str) -> Result<bool> {
    Ok(update_ti(
        conn,
        Sql::bind(
            "\"uninstallState\" = 'skipped', \"uninstallClaimToken\" = NULL, \"uninstallErrorCode\" = ",
            error_code.to_owned(),
        ),
        Sql::bind("ti.\"uninstallState\" = 'pending' AND ti.\"id\" = ", id),
    )? > 0)
}

/// `markCancelled`: a stopped uninstall gives the install back.
pub fn mark_cancelled(conn: &Connection, installation_id: i64) -> Result<bool> {
    Ok(update_ti(
        conn,
        Sql::raw(
            "\"uninstallState\" = 'none', \"uninstallClaimToken\" = NULL, \"uninstallErrorCode\" = 'templateUninstallCancelled'",
        ),
        Sql::bind(
            "ti.\"status\" IN ('completed', 'partial') AND ti.\"uninstallState\" IN ('partial', 'failed', 'skipped') AND ti.\"id\" = ",
            installation_id,
        ),
    )? > 0)
}

/// `markAbandoned`.
pub fn mark_abandoned(
    conn: &Connection,
    id: i64,
    requested_before: i64,
    counts: &serde_json::Value,
    error_code: &str,
) -> Result<bool> {
    let mut cond = Sql::new(
        "ti.\"id\" = ? AND ti.\"uninstallRequestedAt\" < ? AND ",
        vec![Value::Integer(id), Value::Integer(requested_before)],
    );
    cond.append(claimable());
    Ok(update_ti(
        conn,
        Sql::new(
            "\"uninstallState\" = 'failed', \"uninstallClaimToken\" = NULL, \"uninstallErrorCode\" = ?, \"uninstallCounts\" = ?, \"autoUpdate\" = 0",
            vec![Value::Text(error_code.to_owned()), patch::json(counts)],
        ),
        cond,
    )? > 0)
}

/// `listAbandoned`: `(id, workspaceId)`.
pub fn list_abandoned(
    conn: &Connection,
    requested_before: i64,
    limit: i64,
) -> Result<Vec<(i64, i64)>> {
    let mut sql = Sql::bind(
        format!(
            "SELECT ti.\"id\", ti.\"workspaceId\" FROM {TI} ti WHERE ti.\"uninstallRequestedAt\" < "
        ),
        requested_before,
    );
    sql.push(" AND ");
    sql.append(claimable());
    sql.push(" LIMIT ");
    sql.push_bind(limit);
    sql.query_map(conn, |row| Ok((row.get(0)?, row.get(1)?)))
}

/// `isInProgress`: an active uninstall, or a doomed live tracked resource.
pub fn is_in_progress(conn: &Connection, installation_id: i64) -> Result<bool> {
    let active = !Sql::bind(
        format!(
            "SELECT 1 FROM {TI} WHERE \"uninstallState\" IN ('pending', 'running') AND \"id\" = "
        ),
        installation_id,
    )
    .query_map(conn, |row| row.get::<_, i64>(0))?
    .is_empty();
    if active {
        return Ok(true);
    }
    Ok(!Sql::new(
        "SELECT 1 FROM \"TemplateInstalledResource\" WHERE \"installationId\" = ? AND \"uninstallOutcome\" IN (SELECT value FROM json_each(?)) AND \"detachedAt\" IS NULL LIMIT 1",
        vec![
            Value::Integer(installation_id),
            Value::Text(text_list(&TEMPLATE_UNINSTALL_DOOMED)),
        ],
    )
    .query_map(conn, |row| row.get::<_, i64>(0))?
    .is_empty())
}

// ---------------------------------------------------------------------------
// effects.ts
// ---------------------------------------------------------------------------

/// `TEMPLATE_UNINSTALL_MAX_EFFECT_ATTEMPTS`.
pub const TEMPLATE_UNINSTALL_MAX_EFFECT_ATTEMPTS: i64 = 20;

const TIR: &str = "\"TemplateInstalledResource\"";

/// One `UPDATE TemplateInstalledResource SET <col> = <json> WHERE id = ?` per entry (the
/// Postgres `UPDATE .. FROM (VALUES ..)`).
fn set_json_per_row(
    conn: &Connection,
    column: &str,
    entries: &[(i64, serde_json::Value)],
) -> Result<()> {
    let now = now_ms();
    for (id, value) in entries {
        conn.execute(
            &format!("UPDATE {TIR} SET \"{column}\" = ?1, \"updatedAt\" = ?2 WHERE \"id\" = ?3"),
            (value.to_string(), now, id),
        )?;
    }
    Ok(())
}

fn update_tir(conn: &Connection, set: Sql, cond: Sql) -> Result<usize> {
    let mut sql = Sql::raw(format!("UPDATE {TIR} SET "));
    sql.append(set);
    sql.push(", \"updatedAt\" = ");
    sql.push_bind(now_ms());
    sql.push(" WHERE ");
    sql.append(cond);
    sql.execute(conn)
}

fn ids_in(ids: &[i64]) -> Sql {
    Sql::bind(
        "\"id\" IN (SELECT value FROM json_each(",
        super::id_list(ids),
    )
    .closed("))")
}

/// `setEffects`: `(tracked row id, effects array)`.
pub fn set_effects(conn: &Connection, entries: &[(i64, serde_json::Value)]) -> Result<()> {
    set_json_per_row(conn, "uninstallEffects", entries)
}

/// `addInstallationWarning`: appends one warning.
pub fn add_installation_warning(
    conn: &Connection,
    installation_id: i64,
    warning: &serde_json::Value,
) -> Result<()> {
    update_ti(
        conn,
        Sql::bind(
            "\"warningCount\" = \"warningCount\" + 1, \"warnings\" = json_insert(\"warnings\", '$[#]', json(",
            warning.to_string(),
        )
        .closed("))"),
        Sql::bind("ti.\"id\" = ", installation_id),
    )?;
    Ok(())
}

/// `listPendingEffects`: `(id, effects)` of tracked rows with effects.
pub fn list_pending_effects(
    conn: &Connection,
    installation_id: i64,
) -> Result<Vec<(i64, Option<super::Json>)>> {
    Sql::bind(
        format!("SELECT \"id\", \"uninstallEffects\" FROM {TIR} WHERE \"uninstallEffects\" IS NOT NULL AND \"installationId\" = "),
        installation_id,
    )
    .query_map(conn, |row| Ok((row.get(0)?, row.get(1)?)))
}

/// `listInstallationsWithPendingEffects`: `(installationId, workspaceId)`, fewest
/// attempts first.
pub fn list_installations_with_pending_effects(
    conn: &Connection,
    limit: i64,
) -> Result<Vec<(i64, i64)>> {
    Sql::new(
        format!(
            "SELECT \"installationId\", \"workspaceId\" FROM {TIR} WHERE \"uninstallEffects\" IS NOT NULL AND \"uninstallEffectsAttempts\" < ? GROUP BY \"installationId\", \"workspaceId\" ORDER BY min(\"uninstallEffectsAttempts\"), min(\"id\") LIMIT ?"
        ),
        vec![
            Value::Integer(TEMPLATE_UNINSTALL_MAX_EFFECT_ATTEMPTS),
            Value::Integer(limit),
        ],
    )
    .query_map(conn, |row| Ok((row.get(0)?, row.get(1)?)))
}

/// `bumpEffectAttempts`: the ids that reached the attempt cap.
pub fn bump_effect_attempts(conn: &Connection, ids: &[i64]) -> Result<Vec<i64>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut sql = Sql::raw(format!(
        "UPDATE {TIR} SET \"uninstallEffectsAttempts\" = \"uninstallEffectsAttempts\" + 1, \"updatedAt\" = "
    ));
    sql.push_bind(now_ms());
    sql.push(" WHERE ");
    sql.append(ids_in(ids));
    sql.push(" RETURNING \"id\", \"uninstallEffectsAttempts\"");
    let rows: Vec<(i64, i64)> = sql.query_map(conn, |row| Ok((row.get(0)?, row.get(1)?)))?;
    Ok(rows
        .into_iter()
        .filter(|(_, n)| *n >= TEMPLATE_UNINSTALL_MAX_EFFECT_ATTEMPTS)
        .map(|(id, _)| id)
        .collect())
}

/// `clearEffects`.
pub fn clear_effects(conn: &Connection, ids: &[i64]) -> Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    update_tir(conn, Sql::raw("\"uninstallEffects\" = NULL"), ids_in(ids))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// tombstones.ts
// ---------------------------------------------------------------------------

/// `INSTALL_ROOT_PREFIX`.
pub const INSTALL_ROOT_PREFIX: &str = "__installRoot:";

/// `TEMPLATE_UNINSTALL_RETRY_TOMBSTONE`.
pub const TEMPLATE_UNINSTALL_RETRY_TOMBSTONE: [(&str, &str); 6] = [
    ("failed", "deleting"),
    ("failedModified", "deletingModified"),
    ("failedUnverified", "deletingUnverified"),
    ("blockedByReference", "deleting"),
    ("blockedByReferenceModified", "deletingModified"),
    ("blockedByReferenceUnverified", "deletingUnverified"),
];

/// `TEMPLATE_UNINSTALL_COUNT_KEY_BY_OUTCOME`.
pub fn count_key_by_outcome(outcome: &str) -> Option<&'static str> {
    Some(match outcome {
        "deleted" => "deleted",
        "deletedModified" => "deletedModified",
        "deletedUnverified" => "deletedUnverified",
        "keptHasDependents" => "keptHasDependents",
        "keptPartlyRemoved" => "keptPartlyRemoved",
        "keptShared" => "keptShared",
        "keptLockedByOther" => "keptLockedByOther",
        "absent" => "absent",
        "failed" | "failedModified" | "failedUnverified" => "failed",
        "blockedByReference" | "blockedByReferenceModified" | "blockedByReferenceUnverified" => {
            "blockedByReference"
        }
        _ => return None,
    })
}

/// `countOutcomes`: the counts of the settled outcomes (install-root folders left out).
pub fn count_outcomes(conn: &Connection, installation_id: i64) -> Result<serde_json::Value> {
    let rows: Vec<(Option<String>, i64)> = Sql::new(
        format!(
            "SELECT \"uninstallOutcome\", count(*) FROM {TIR} WHERE \"installationId\" = ? AND NOT (\"resourceKind\" = 'folder' AND substr(\"sourceResourceId\", 1, length(?)) = ?) GROUP BY \"uninstallOutcome\""
        ),
        vec![
            Value::Integer(installation_id),
            Value::Text(INSTALL_ROOT_PREFIX.to_owned()),
            Value::Text(INSTALL_ROOT_PREFIX.to_owned()),
        ],
    )
    .query_map(conn, |row| Ok((row.get(0)?, row.get(1)?)))?;
    let mut counts = empty_counts();
    let map = counts.as_object_mut().expect("counts object");
    let mut add = |key: &str, n: i64| {
        let cur = map
            .get(key)
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0);
        map.insert(key.to_owned(), serde_json::Value::from(cur + n));
    };
    for (outcome, n) in rows {
        let Some(outcome) = outcome else { continue };
        let Some(key) = count_key_by_outcome(&outcome) else {
            continue;
        };
        add(key, n);
        if outcome == "deletedModified" || outcome == "deletedUnverified" {
            add("deleted", n);
        }
    }
    Ok(counts)
}

/// `listTombstones`: by id.
pub fn list_tombstones(
    conn: &Connection,
    installation_id: i64,
    limit: i64,
) -> Result<Vec<super::models::TemplateInstalledResource>> {
    Sql::new(
        format!(
            "{} WHERE \"installationId\" = ? AND \"uninstallOutcome\" IN (SELECT value FROM json_each(?)) ORDER BY \"id\" LIMIT ?",
            super::models::TemplateInstalledResource::select_sql()
        ),
        vec![
            Value::Integer(installation_id),
            Value::Text(text_list(&super::template_installed_resource_uninstall::TEMPLATE_UNINSTALL_TOMBSTONES)),
            Value::Integer(limit),
        ],
    )
    .models(conn)
}

/// `setOutcomes`: `effects` `None` leaves the column (`Some(None)` clears it).
pub fn set_outcomes(
    conn: &Connection,
    ids: &[i64],
    outcome: &str,
    effects: Option<Option<&serde_json::Value>>,
    detach: bool,
) -> Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    let mut set = Sql::bind("\"uninstallOutcome\" = ", outcome.to_owned());
    if let Some(effects) = effects {
        set.push(", \"uninstallEffects\" = ");
        set.push_bind(effects.map_or(Value::Null, patch::json));
    }
    if detach {
        set.push(", \"detachedAt\" = ");
        set.push_bind(now_ms());
    }
    update_tir(conn, set, ids_in(ids))?;
    Ok(())
}

/// `clearFrozenFrom`.
pub fn clear_frozen_from(conn: &Connection, ids: &[i64]) -> Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    update_tir(
        conn,
        Sql::raw("\"uninstallFrozenFrom\" = NULL"),
        ids_in(ids),
    )?;
    Ok(())
}

/// `setFrozenFrom`: `(tracked row id, flags object)`.
pub fn set_frozen_from(conn: &Connection, entries: &[(i64, serde_json::Value)]) -> Result<()> {
    set_json_per_row(conn, "uninstallFrozenFrom", entries)
}

/// `markDeleteCommitted`.
pub fn mark_delete_committed(conn: &Connection, ids: &[i64]) -> Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    let mut cond = ids_in(ids);
    cond.push(" AND \"uninstallDrainStartedAt\" IS NULL");
    update_tir(
        conn,
        Sql::bind("\"uninstallDrainStartedAt\" = ", now_ms()),
        cond,
    )?;
    Ok(())
}

fn doomed_live(installation_id: i64) -> Sql {
    Sql::new(
        "\"installationId\" = ? AND \"uninstallOutcome\" IN (SELECT value FROM json_each(?)) AND \"detachedAt\" IS NULL",
        vec![
            Value::Integer(installation_id),
            Value::Text(text_list(&TEMPLATE_UNINSTALL_DOOMED)),
        ],
    )
}

/// `untombstone`: doomed live rows whose drain has not started go back to no outcome.
pub fn untombstone(conn: &Connection, installation_id: i64) -> Result<()> {
    let mut cond = doomed_live(installation_id);
    cond.push(" AND \"uninstallDrainStartedAt\" IS NULL");
    update_tir(
        conn,
        Sql::raw(
            "\"uninstallOutcome\" = NULL, \"uninstallDrainStartedAt\" = NULL, \"uninstallFrozenFrom\" = NULL",
        ),
        cond,
    )?;
    Ok(())
}

/// `listDoomed`.
pub fn list_doomed(
    conn: &Connection,
    installation_id: i64,
) -> Result<Vec<super::models::TemplateInstalledResource>> {
    let mut sql = Sql::raw(format!(
        "{} WHERE ",
        super::models::TemplateInstalledResource::select_sql()
    ));
    sql.append(doomed_live(installation_id));
    sql.models(conn)
}

/// `resetFailedToDeleting`: failed / blocked outcomes go back to their tombstone.
pub fn reset_failed_to_deleting(conn: &Connection, installation_id: i64) -> Result<()> {
    let mut cases = String::from("CASE \"uninstallOutcome\"");
    for (from, to) in TEMPLATE_UNINSTALL_RETRY_TOMBSTONE {
        cases.push_str(&format!(" WHEN '{from}' THEN '{to}'"));
    }
    cases.push_str(" END");
    let froms: Vec<&str> = TEMPLATE_UNINSTALL_RETRY_TOMBSTONE
        .iter()
        .map(|(f, _)| *f)
        .collect();
    update_tir(
        conn,
        Sql::raw(format!("\"uninstallOutcome\" = {cases}")),
        Sql::new(
            "\"installationId\" = ? AND \"uninstallOutcome\" IN (SELECT value FROM json_each(?))",
            vec![
                Value::Integer(installation_id),
                Value::Text(text_list(&froms)),
            ],
        ),
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// install-roots.ts
// ---------------------------------------------------------------------------

/// `InstallFolderRow`.
#[derive(Debug, Clone, PartialEq)]
pub struct InstallFolderRow {
    pub folder_id: i64,
    pub workspace_id: i64,
    pub tracked_row_id: Option<i64>,
}

/// `findInstallRoots`: the folders an install created as its roots (tracked install-root
/// folders first, in row order, then the installation's own folder).
pub fn find_install_roots(
    conn: &Connection,
    installation_id: i64,
) -> Result<Vec<InstallFolderRow>> {
    let installation: Option<(Option<i64>, i64)> = Sql::bind(
        format!("SELECT \"installFolderId\", \"workspaceId\" FROM {TI} WHERE \"id\" = "),
        installation_id,
    )
    .query_map(conn, |row| Ok((row.get(0)?, row.get(1)?)))?
    .into_iter()
    .next();
    let Some((folder_id, workspace_id)) = installation else {
        return Ok(Vec::new());
    };
    let tracked: Vec<(i64, i64, String, bool)> = Sql::bind(
        format!("SELECT \"id\", \"resourceId\", \"sourceResourceId\", \"wasExisting\" FROM {TIR} WHERE \"resourceKind\" = 'folder' AND \"installationId\" = "),
        installation_id,
    )
    .query_map(conn, |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)))?;
    let mut roots: Vec<InstallFolderRow> = Vec::new();
    for (id, resource_id, source, was_existing) in &tracked {
        if !was_existing && source.starts_with(INSTALL_ROOT_PREFIX) {
            let row = InstallFolderRow {
                folder_id: *resource_id,
                workspace_id,
                tracked_row_id: Some(*id),
            };
            match roots.iter_mut().find(|r| r.folder_id == *resource_id) {
                Some(existing) => *existing = row,
                None => roots.push(row),
            }
        }
    }
    if let Some(folder_id) = folder_id
        && !roots.iter().any(|r| r.folder_id == folder_id)
    {
        let own = tracked.iter().find(|r| r.1 == folder_id).map(|r| r.0);
        roots.push(InstallFolderRow {
            folder_id,
            workspace_id,
            tracked_row_id: own,
        });
    }
    Ok(roots)
}

/// `listFolderReferenceColumns`: `(table, column)` of every foreign key to `Folder`,
/// except `Folder` itself and `TemplateInstallation` (read from the SQLite schema).
fn list_folder_reference_columns(conn: &Connection) -> Result<Vec<(String, String)>> {
    Sql::raw(
        "SELECT m.\"name\", f.\"from\" FROM sqlite_schema m, pragma_foreign_key_list(m.\"name\") f WHERE m.\"type\" = 'table' AND f.\"table\" = 'Folder' AND m.\"name\" NOT IN ('Folder', 'TemplateInstallation') ORDER BY m.\"name\", f.\"from\"",
    )
    .query_map(conn, |row| Ok((row.get(0)?, row.get(1)?)))
}

/// `deleteFolderIfEmpty`: `deleted`, `notEmpty` (a child folder or a referencing row) or
/// `absent`. Run it in one write job.
pub fn delete_folder_if_empty(
    conn: &Connection,
    workspace_id: i64,
    folder_id: i64,
) -> Result<&'static str> {
    let exists = !Sql::new(
        "SELECT \"id\" FROM \"Folder\" WHERE \"id\" = ? AND \"workspaceId\" = ?",
        vec![Value::Integer(folder_id), Value::Integer(workspace_id)],
    )
    .query_map(conn, |row| row.get::<_, i64>(0))?
    .is_empty();
    if !exists {
        return Ok("absent");
    }
    let has_child = !Sql::new(
        "SELECT 1 FROM \"Folder\" WHERE \"workspaceId\" = ? AND EXISTS (SELECT 1 FROM json_each(\"paths\") WHERE CAST(value AS INTEGER) = ?) AND \"id\" <> ? LIMIT 1",
        vec![
            Value::Integer(workspace_id),
            Value::Integer(folder_id),
            Value::Integer(folder_id),
        ],
    )
    .query_map(conn, |row| row.get::<_, i64>(0))?
    .is_empty();
    if has_child {
        return Ok("notEmpty");
    }
    for (table, column) in list_folder_reference_columns(conn)? {
        let table = table.replace('"', "\"\"");
        let column = column.replace('"', "\"\"");
        let held = !Sql::bind(
            format!("SELECT 1 FROM \"{table}\" WHERE \"{column}\" = "),
            folder_id,
        )
        .closed(" LIMIT 1")
        .query_map(conn, |row| row.get::<_, i64>(0))?
        .is_empty();
        if held {
            return Ok("notEmpty");
        }
    }
    conn.execute(
        "DELETE FROM \"Folder\" WHERE \"id\" = ?1 AND \"workspaceId\" = ?2",
        (folder_id, workspace_id),
    )?;
    Ok("deleted")
}

// ---------------------------------------------------------------------------
// references.ts (`templateUninstallReferenceRepository`)
// ---------------------------------------------------------------------------
//
// The Postgres `regexp_matches(.., 'g')` scans run in Rust (`regex` crate) over the rows'
// text; the prefix pattern's lookahead `(?=[^0-9]|$)` is a check of the next character.
// A JSON column's text is the stored compact JSON (the patterns allow `": ?"`). Results
// are de-duplicated per probe (`DISTINCT` / `UNION`) and sorted by `(parent, holder)`.
// `restrictSql` is raw SQLite SQL from the caller.

/// `ReferenceSlots`.
#[derive(Debug, Clone, Default)]
pub struct ReferenceSlots {
    pub scalars: Vec<String>,
    pub arrays: Vec<String>,
    pub prefixes: Vec<String>,
    pub condition_types: Vec<String>,
}

/// `ReferenceColumnProbe`.
#[derive(Debug, Clone, Default)]
pub struct ReferenceColumnProbe {
    pub holder_kind: Option<String>,
    pub table: String,
    pub column: String,
    pub holder_id_column: String,
    pub workspace_column: Option<String>,
    pub text_column: bool,
}

/// `ReferenceContainerProbe`.
#[derive(Debug, Clone, Default)]
pub struct ReferenceContainerProbe {
    pub holder_kind: Option<String>,
    pub table: String,
    pub holder_id_column: String,
    pub json_columns: Vec<String>,
    pub workspace_column: Option<String>,
    pub scope_through_broadcast: bool,
    pub condition_type_column: Option<String>,
    pub contact_filter_column: Option<String>,
    pub restrict_sql: Option<String>,
}

/// `InboundEdge`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct InboundEdge {
    pub parent: String,
    pub holder_kind: Option<String>,
    pub holder_id: Option<String>,
}

fn ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// `idAlternation`: longest first, then by code unit.
fn id_alternation(ids: &[String]) -> String {
    let mut sorted: Vec<&String> = ids.iter().collect();
    sorted.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
    sorted
        .iter()
        .map(|s| s.as_str())
        .collect::<Vec<_>>()
        .join("|")
}

fn scoped_conditions() -> String {
    "(SELECT c.* FROM \"Condition\" c WHERE c.\"triggerId\" IN (SELECT tr.\"id\" FROM \"Trigger\" tr WHERE tr.\"workspaceId\" = ?) UNION ALL SELECT c.* FROM \"Condition\" c WHERE c.\"webhookId\" IN (SELECT wh.\"id\" FROM \"Webhook\" wh WHERE wh.\"workspaceId\" = ?))".to_owned()
}

fn to_edges(
    pairs: BTreeSet<(String, Option<String>)>,
    holder_kind: &Option<String>,
) -> Vec<InboundEdge> {
    pairs
        .into_iter()
        .map(|(parent, holder_id)| InboundEdge {
            parent,
            holder_kind: holder_kind.clone(),
            holder_id,
        })
        .collect()
}

fn regex(pattern: &str) -> Result<regex::Regex> {
    regex::Regex::new(pattern).map_err(|e| Error::msg(e.to_string()))
}

/// Every capture-1 text of `re` in `text` (`regexp_matches(.., 'g')`); `digit_guard`
/// rejects a match followed by an ASCII digit (and retries one character later).
fn matches_g(re: &regex::Regex, text: &str, digit_guard: bool) -> Vec<String> {
    let mut out = Vec::new();
    let mut pos = 0;
    while pos <= text.len() {
        let Some(caps) = re.captures_at(text, pos) else {
            break;
        };
        let whole = caps.get(0).expect("group 0");
        let accepted = !digit_guard
            || !text[whole.end()..]
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_digit());
        if accepted {
            if let Some(m) = caps.get(1) {
                out.push(m.as_str().to_owned());
            }
            pos = if whole.end() > whole.start() {
                whole.end()
            } else {
                whole.end() + text[whole.end()..].chars().next().map_or(1, char::len_utf8)
            };
        } else {
            pos = whole.start()
                + text[whole.start()..]
                    .chars()
                    .next()
                    .map_or(1, char::len_utf8);
        }
    }
    out
}

fn find_column_edges(
    conn: &Connection,
    probe: &ReferenceColumnProbe,
    parent_ids: &[String],
    workspace_id: i64,
) -> Result<Vec<InboundEdge>> {
    let is_condition = probe.table == "Condition";
    let mut sql = Sql::default();
    let column = format!("t.{}", ident(&probe.column));
    let holder = if probe.holder_kind.is_some() {
        format!("CAST(t.{} AS TEXT)", ident(&probe.holder_id_column))
    } else {
        "NULL".to_owned()
    };
    sql.push(&format!(
        "SELECT DISTINCT CAST({column} AS TEXT), {holder} FROM "
    ));
    if is_condition {
        sql.push(&scoped_conditions());
        sql.params.push(Value::Integer(workspace_id));
        sql.params.push(Value::Integer(workspace_id));
        sql.push(" AS t");
    } else {
        sql.push(&format!("{} AS t", ident(&probe.table)));
    }
    let ids = if probe.text_column {
        text_list(parent_ids)
    } else {
        let nums: Vec<i64> = parent_ids.iter().filter_map(|s| s.parse().ok()).collect();
        super::id_list(&nums)
    };
    sql.push(&format!(" WHERE {column} IN (SELECT value FROM json_each("));
    sql.push_bind(ids);
    sql.push("))");
    if let (Some(ws), false) = (&probe.workspace_column, is_condition) {
        sql.push(&format!(" AND t.{} = ", ident(ws)));
        sql.push_bind(workspace_id);
    }
    if is_condition && probe.column == "sourceId" {
        sql.push(" AND t.\"type\" = 'trigger'");
    }
    if probe.holder_kind.is_some() {
        sql.push(&format!(
            " AND CAST(t.{} AS TEXT) IS NOT CAST({column} AS TEXT)",
            ident(&probe.holder_id_column)
        ));
    }
    let rows: Vec<(Option<String>, Option<String>)> =
        sql.query_map(conn, |row| Ok((row.get(0)?, row.get(1)?)))?;
    let mut pairs = BTreeSet::new();
    for (parent, holder_id) in rows {
        if let Some(parent) = parent {
            pairs.insert((parent, holder_id));
        }
    }
    Ok(to_edges(pairs, &probe.holder_kind))
}

/// A container row: the holder id and the text of each column.
type ContainerRow = (Option<String>, Vec<Option<String>>);

/// `containerSource`: `FROM` and `WHERE` of a container probe.
fn container_rows(
    conn: &Connection,
    container: &ReferenceContainerProbe,
    columns: &[String],
    slots: &ReferenceSlots,
    workspace_id: i64,
) -> Result<Vec<ContainerRow>> {
    let holder = if container.holder_kind.is_some() {
        format!("CAST(t.{} AS TEXT)", ident(&container.holder_id_column))
    } else {
        "NULL".to_owned()
    };
    let cols: Vec<String> = columns
        .iter()
        .map(|c| format!("CAST(t.{} AS TEXT)", ident(c)))
        .collect();
    let mut sql = Sql::raw(format!("SELECT {holder}, {} FROM ", cols.join(", ")));
    let restrict = container
        .restrict_sql
        .as_ref()
        .map_or(String::new(), |r| format!(" AND ({r})"));
    if let Some(ws) = &container.workspace_column {
        sql.push(&format!(
            "{} AS t WHERE t.{} = ",
            ident(&container.table),
            ident(ws)
        ));
        sql.push_bind(workspace_id);
    } else if container.scope_through_broadcast {
        sql.push(&format!(
            "{} AS t WHERE EXISTS (SELECT 1 FROM \"Broadcast\" bc WHERE bc.\"id\" = t.\"broadcastId\" AND bc.\"workspaceId\" = ",
            ident(&container.table)
        ));
        sql.push_bind(workspace_id);
        sql.push(" AND bc.\"deletedAt\" IS NULL)");
    } else {
        sql.push(&scoped_conditions());
        sql.params.push(Value::Integer(workspace_id));
        sql.params.push(Value::Integer(workspace_id));
        sql.push(" AS t WHERE 1");
    }
    sql.push(&restrict);
    if let Some(ct) = &container.condition_type_column {
        sql.push(&format!(
            " AND t.{} IN (SELECT value FROM json_each(",
            ident(ct)
        ));
        sql.push_bind(text_list(&slots.condition_types));
        sql.push("))");
    }
    let n = columns.len();
    sql.query_map(conn, |row| {
        let holder: Option<String> = row.get(0)?;
        let mut texts = Vec::with_capacity(n);
        for i in 0..n {
            texts.push(row.get::<_, Option<String>>(i + 1)?);
        }
        Ok((holder, texts))
    })
}

fn find_field_edges(
    conn: &Connection,
    container: &ReferenceContainerProbe,
    slots: &ReferenceSlots,
    parent_ids: &[String],
    workspace_id: i64,
) -> Result<Vec<InboundEdge>> {
    let alternation = id_alternation(parent_ids);
    let quoted = regex(&format!("\"({alternation})\""))?;
    let mut simple: Vec<(regex::Regex, bool)> = Vec::new();
    let mut nested: Vec<regex::Regex> = Vec::new();
    if container.condition_type_column.is_some() {
        simple.push((quoted.clone(), false));
    } else {
        if !slots.scalars.is_empty() {
            simple.push((
                regex(&format!(
                    "\"(?:{})\": ?\"({alternation})\"",
                    slots.scalars.join("|")
                ))?,
                false,
            ));
        }
        if !slots.prefixes.is_empty() {
            simple.push((
                regex(&format!(
                    "(?:^|[^A-Za-z0-9])(?:{}):({alternation})",
                    slots.prefixes.join("|")
                ))?,
                true,
            ));
        }
        if !slots.arrays.is_empty() {
            nested.push(regex(&format!(
                "\"(?:{})\": ?\\[([^\\]]*)\\]",
                slots.arrays.join("|")
            ))?);
        }
        if !slots.condition_types.is_empty() {
            let field = format!("\"field\": ?\"(?:{})\"", slots.condition_types.join("|"));
            let value = "\"value\": ?(\\[[^\\]]*\\]|\"[^\"]*\")";
            nested.push(regex(&format!("{field}[^{{}}]*{value}"))?);
            nested.push(regex(&format!("{value}[^{{}}]*{field}"))?);
        }
    }
    if simple.is_empty() && nested.is_empty() {
        return Ok(Vec::new());
    }
    let rows = container_rows(
        conn,
        container,
        &container.json_columns,
        slots,
        workspace_id,
    )?;
    let mut pairs = BTreeSet::new();
    for (holder, texts) in rows {
        let Some(texts) = texts.into_iter().collect::<Option<Vec<String>>>() else {
            continue;
        };
        let text = texts
            .iter()
            .map(|t| t.replace('\\', ""))
            .collect::<Vec<_>>()
            .join(" ");
        let mut found = Vec::new();
        for (re, guard) in &simple {
            found.extend(matches_g(re, &text, *guard));
        }
        for re in &nested {
            for body in matches_g(re, &text, false) {
                found.extend(matches_g(&quoted, &body, false));
            }
        }
        for parent in found {
            if container.holder_kind.is_some() && holder.as_deref() == Some(parent.as_str()) {
                continue;
            }
            pairs.insert((parent, holder.clone()));
        }
    }
    Ok(to_edges(pairs, &container.holder_kind))
}

fn find_contact_filter_edges(
    conn: &Connection,
    container: &ReferenceContainerProbe,
    slots: &ReferenceSlots,
    parent_ids: &[String],
    workspace_id: i64,
) -> Result<Vec<InboundEdge>> {
    let Some(filter_column) = &container.contact_filter_column else {
        return Ok(Vec::new());
    };
    if slots.condition_types.is_empty() {
        return Ok(Vec::new());
    }
    let quoted = regex(&format!("\"({})\"", id_alternation(parent_ids)))?;
    let mut probe = container.clone();
    probe.condition_type_column = None;
    let rows = container_rows(
        conn,
        &probe,
        std::slice::from_ref(filter_column),
        slots,
        workspace_id,
    )?;
    let mut pairs = BTreeSet::new();
    for (holder, texts) in rows {
        let Some(Some(text)) = texts.into_iter().next() else {
            continue;
        };
        let Ok(filter) = serde_json::from_str::<serde_json::Value>(&text) else {
            continue;
        };
        let Some(conditions) = filter
            .get("conditions")
            .and_then(serde_json::Value::as_array)
        else {
            continue;
        };
        for c in conditions {
            let field = c.get("field").and_then(serde_json::Value::as_str);
            if !field.is_some_and(|f| slots.condition_types.iter().any(|t| t == f)) {
                continue;
            }
            let Some(value) = c.get("value") else {
                continue;
            };
            let value_text = value.to_string().replace('\\', "");
            for parent in matches_g(&quoted, &value_text, false) {
                pairs.insert((parent, holder.clone()));
            }
        }
    }
    Ok(to_edges(pairs, &container.holder_kind))
}

/// `findInboundEdges`: who still references these parent ids (column probes, then
/// container probes). Fails on a non-numeric parent id.
pub fn find_inbound_edges(
    conn: &Connection,
    workspace_id: i64,
    parent_ids: &[String],
    column_probes: &[ReferenceColumnProbe],
    container_probes: &[ReferenceContainerProbe],
    slots: &ReferenceSlots,
) -> Result<Vec<InboundEdge>> {
    if parent_ids.is_empty() {
        return Ok(Vec::new());
    }
    for id in parent_ids {
        if id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()) {
            return Err(Error::msg(format!(
                "Non-numeric id in a reference probe: {id}"
            )));
        }
    }
    let mut edges = Vec::new();
    for probe in column_probes {
        edges.extend(find_column_edges(conn, probe, parent_ids, workspace_id)?);
    }
    for container in container_probes {
        if container.contact_filter_column.is_some() {
            edges.extend(find_contact_filter_edges(
                conn,
                container,
                slots,
                parent_ids,
                workspace_id,
            )?);
            continue;
        }
        if container.condition_type_column.is_some() && slots.condition_types.is_empty() {
            continue;
        }
        edges.extend(find_field_edges(
            conn,
            container,
            slots,
            parent_ids,
            workspace_id,
        )?);
    }
    Ok(edges)
}
