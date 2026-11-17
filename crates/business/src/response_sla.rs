//! `packages/business/src/response-sla/service.ts`: the workspace policy.
//! ponytail: no read cache (one process, SQLite reads are cheap). The window
//! snapshots (`trackWindow`, `computeSnapshot`) wait for the working-hours
//! domain (Q.D9): the deadline is counted in the agent's working time.

use chatbotx_db::Db;
use chatbotx_db::repo::models::ResponseSlaPolicy;
use chatbotx_db::repo::response_sla::{self as repo, ResponseSlaPolicyInput};
use serde_json::{Value, json};

use crate::errors::{BusinessError, Result};
use crate::tx;

pub const MIN_TARGET_SECONDS: i64 = 60;
pub const MAX_TARGET_SECONDS: i64 = 30 * 24 * 60 * 60;

/// `toResponseSlaResource`.
pub fn resource(row: &ResponseSlaPolicy) -> Value {
    json!({
        "enabled": row.enabled,
        "firstResponseTargetSeconds": row.first_response_target_seconds,
        "nextResponseTargetSeconds": row.next_response_target_seconds,
        "breachingThresholdPercent": row.breaching_threshold_percent,
        "createdAt": chatbotx_db::repo::iso(row.created_at),
        "updatedAt": chatbotx_db::repo::iso(row.updated_at),
    })
}

/// `parsePolicy`.
fn parse_policy(data: &ResponseSlaPolicyInput) -> Result<()> {
    let in_range = |v: i64| (MIN_TARGET_SECONDS..=MAX_TARGET_SECONDS).contains(&v);
    if !in_range(data.first_response_target_seconds) {
        return Err(BusinessError::validation(
            "firstResponseTargetSeconds",
            format!("Must be a whole number of seconds between {MIN_TARGET_SECONDS} and {MAX_TARGET_SECONDS}"),
        ));
    }
    if data.next_response_target_seconds.is_some_and(|v| !in_range(v)) {
        return Err(BusinessError::validation(
            "nextResponseTargetSeconds",
            format!("Must be null or a whole number of seconds between {MIN_TARGET_SECONDS} and {MAX_TARGET_SECONDS}"),
        ));
    }
    if !(1..=99).contains(&data.breaching_threshold_percent) {
        return Err(BusinessError::validation(
            "breachingThresholdPercent",
            "Must be a whole number between 1 and 99",
        ));
    }
    Ok(())
}

/// `findPolicy`.
pub async fn find_policy(db: &Db, workspace_id: i64) -> Result<Option<ResponseSlaPolicy>> {
    tx::read(db, move |c| Ok(repo::find_policy(c, workspace_id)?)).await
}

/// `upsertPolicy`: a disabled policy clears the open windows' deadlines.
/// Returns the row and the audit detail.
pub async fn upsert_policy(
    db: &Db,
    workspace_id: i64,
    data: ResponseSlaPolicyInput,
) -> Result<(ResponseSlaPolicy, &'static str)> {
    parse_policy(&data)?;
    let row = tx::write(db, move |c| {
        let saved = repo::upsert_policy(c, workspace_id, data)?;
        if !saved.enabled {
            repo::clear_workspace_snapshots(c, workspace_id)?;
        }
        Ok(saved)
    })
    .await?;
    let detail = if row.enabled {
        "updated the response SLA policy"
    } else {
        "disabled the response SLA policy"
    };
    Ok((row, detail))
}

/// `deletePolicy`: `true` when a policy was removed.
pub async fn delete_policy(db: &Db, workspace_id: i64) -> Result<bool> {
    tx::write(db, move |c| {
        let removed = repo::delete_policy(c, workspace_id)?;
        repo::clear_workspace_snapshots(c, workspace_id)?;
        Ok(removed)
    })
    .await
}
