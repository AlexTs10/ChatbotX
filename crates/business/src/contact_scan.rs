//! `contact-scan/service.ts` (`contactScanService`): `schedule`,
//! `getStatus`, `listHistory` over `CoexistSyncRun` rows of
//! `type = 'contact_scan'`, with `resolveContactScanAvailability`.

use chatbotx_db::Db;
use chatbotx_db::repo::{iso, new_id, now_ms};
use chatbotx_db::rusqlite::{Connection, OptionalExtension as _, Row};
use serde_json::{Value, json};

use crate::errors::{BusinessError, Result};
use crate::tx;

/// `CONTACT_SCAN_COOLDOWN_MS` / `CONTACT_SCAN_ETA_MS`.
const COOLDOWN_MS: i64 = 24 * 60 * 60 * 1000;
const ETA_MS: i64 = 4 * 60 * 60 * 1000;
/// `CONTACT_SCAN_TRIGGER_SOURCE`.
const TRIGGER_SOURCE: &str = "contact-scan-manual";

fn repo_err(error: chatbotx_db::rusqlite::Error) -> BusinessError {
    BusinessError::Repo(error.into())
}

fn exception(message: &str, code: &str, status: u16) -> BusinessError {
    BusinessError::exception(message, code, status)
}

const COLUMNS: &str = r#""id", "channel", "status", "scanFromAt", "importedContactCount", "currentScan",
    "startedAt", "finishedAt", "createdAt", "requestedByUserId", "currentError""#;

/// A `CoexistSyncRun` row of the scan history.
#[derive(Debug, Clone)]
struct Run {
    id: i64,
    channel: String,
    status: String,
    scan_from_at: Option<i64>,
    imported: i64,
    current_scan: i64,
    started_at: Option<i64>,
    finished_at: Option<i64>,
    created_at: i64,
    requested_by: Option<i64>,
    current_error: Option<String>,
}

fn run(r: &Row<'_>) -> chatbotx_db::rusqlite::Result<Run> {
    Ok(Run {
        id: r.get(0)?,
        channel: r.get(1)?,
        status: r.get(2)?,
        scan_from_at: r.get(3)?,
        imported: r.get(4)?,
        current_scan: r.get(5)?,
        started_at: r.get(6)?,
        finished_at: r.get(7)?,
        created_at: r.get(8)?,
        requested_by: r.get(9)?,
        current_error: r.get(10)?,
    })
}

fn date(ms: Option<i64>) -> Value {
    ms.map_or(Value::Null, |ms| json!(iso(ms)))
}

/// `toRunView`.
fn run_view(run: &Run) -> Value {
    json!({
        "id": run.id.to_string(),
        "status": run.status,
        "scanFromAt": date(run.scan_from_at),
        "createdAt": iso(run.created_at),
        "startedAt": date(run.started_at),
        "finishedAt": date(run.finished_at),
        "importedContactCount": run.imported,
        "currentScan": run.current_scan,
        "currentError": run.current_error,
    })
}

/// `contactScanRunPublicResource`.
fn history_item(run: &Run) -> Value {
    let mut view = run_view(run);
    view["channel"] = json!(run.channel);
    view["requestedByUserId"] = run
        .requested_by
        .map_or(Value::Null, |id| json!(id.to_string()));
    view
}

/// `resolveContactScanAvailability`.
fn availability(latest: Option<&Run>, now: i64) -> Value {
    let Some(latest) = latest else {
        return json!({ "canScan": true });
    };
    let next = (latest.created_at + COOLDOWN_MS).max(latest.created_at + ETA_MS);
    let active = matches!(latest.status.as_str(), "init" | "running" | "waiting");
    if !active && now - latest.created_at >= COOLDOWN_MS {
        return json!({ "canScan": true });
    }
    json!({
        "canScan": false,
        "blockedReason": if active { "running" } else { "cooldown" },
        "nextScanAt": iso(next),
    })
}

/// The inbox's `(channel, status, integrationId)` (`findWithIntegrations` +
/// `contactScanIntegrationRefs`); `None` = not in the workspace.
fn inbox(
    conn: &Connection,
    workspace_id: i64,
    inbox_id: i64,
) -> Result<Option<(String, String, Option<i64>)>> {
    conn.query_row(
        r#"SELECT i."channel", i."status",
                  CASE i."channel"
                    WHEN 'messenger' THEN (SELECT m."id" FROM "IntegrationMessenger" m WHERE m."inboxId" = i."id")
                    WHEN 'instagram' THEN (SELECT g."id" FROM "IntegrationInstagram" g WHERE g."inboxId" = i."id")
                  END
           FROM "Inbox" i WHERE i."id" = ?1 AND i."workspaceId" = ?2"#,
        [inbox_id, workspace_id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )
    .optional()
    .map_err(repo_err)
}

/// `findLatestContactScanRun`.
fn latest(conn: &Connection, workspace_id: i64, integration_id: i64) -> Result<Option<Run>> {
    conn.query_row(
        &format!(
            r#"SELECT {COLUMNS} FROM "CoexistSyncRun"
               WHERE "workspaceId" = ?1 AND "integrationId" = ?2 AND "type" = 'contact_scan'
               ORDER BY "createdAt" DESC, "id" DESC LIMIT 1"#
        ),
        [workspace_id, integration_id],
        run,
    )
    .optional()
    .map_err(repo_err)
}

fn is_scan_channel(channel: &str) -> bool {
    matches!(channel, "messenger" | "instagram")
}

/// `schedule`: the run id of a new `init` run (`requestedByUserId` = the
/// signed-in user of the session action; the public route passes `None`).
pub async fn schedule(
    db: &Db,
    workspace_id: i64,
    inbox_id: i64,
    scan_from_at: i64,
    requested_by: Option<i64>,
) -> Result<i64> {
    let now = now_ms();
    if scan_from_at >= now {
        return Err(exception(
            "Scan-from time must be in the past.",
            "contactScanFromTimeInvalid",
            400,
        ));
    }
    tx::write(db, move |conn| {
        let (channel, status, integration_id) = inbox(conn, workspace_id, inbox_id)?
            .ok_or_else(|| exception("Inbox not found.", "contactScanInboxNotFound", 404))?;
        if !is_scan_channel(&channel) {
            return Err(exception(
                "This channel does not support Automatic Customer Scan.",
                "contactScanChannelUnsupported",
                400,
            ));
        }
        let Some(integration_id) = integration_id.filter(|_| status == "connected") else {
            return Err(exception("This inbox is not connected.", "contactScanIntegrationDisconnected", 400));
        };
        let running = || exception("A scan is already running for this inbox.", "contactScanAlreadyRunning", 409);
        let last = latest(conn, workspace_id, integration_id)?;
        let open = availability(last.as_ref(), now);
        match open["blockedReason"].as_str() {
            Some("running") => return Err(running()),
            Some(_) => {
                return Err(exception(
                    "This inbox was scanned recently. Please wait before scanning again.",
                    "contactScanCooldown",
                    409,
                ));
            }
            None => {}
        }
        let id = new_id();
        let inserted = conn
            .execute(
                r#"INSERT INTO "CoexistSyncRun" ("id", "createdAt", "updatedAt", "workspaceId", "integrationId",
                     "channel", "type", "status", "triggerSource", "scanFromAt", "requestedByUserId")
                   VALUES (?1, ?2, ?2, ?3, ?4, ?5, 'contact_scan', 'init', ?6, ?7, ?8)
                   ON CONFLICT DO NOTHING"#,
                chatbotx_db::rusqlite::params![id, now, workspace_id, integration_id, channel, TRIGGER_SOURCE, scan_from_at, requested_by],
            )
            .map_err(repo_err)?;
        if inserted == 0 {
            return Err(running());
        }
        Ok(id)
    })
    .await
}

/// `getStatus`.
pub async fn status(db: &Db, workspace_id: i64, inbox_id: i64) -> Result<Value> {
    let now = now_ms();
    tx::read(db, move |conn| {
        let idle = json!({ "status": "idle", "latest": null, "availability": { "canScan": true } });
        let Some((channel, _, Some(integration_id))) = inbox(conn, workspace_id, inbox_id)? else {
            return Ok(idle);
        };
        if !is_scan_channel(&channel) {
            return Ok(idle);
        }
        let last = latest(conn, workspace_id, integration_id)?;
        Ok(json!({
            "status": last.as_ref().map_or("idle", |r| r.status.as_str()),
            "latest": last.as_ref().map_or(Value::Null, run_view),
            "availability": availability(last.as_ref(), now),
        }))
    })
    .await
}

/// `listHistory` (newest first): `{ data, pageCount }`.
pub async fn list(
    db: &Db,
    workspace_id: i64,
    integration_id: Option<i64>,
    page: i64,
    per_page: i64,
) -> Result<Value> {
    tx::read(db, move |conn| {
        let filter = if integration_id.is_some() { r#" AND "integrationId" = ?2"# } else { "" };
        let integration = integration_id.unwrap_or(0);
        let total: i64 = conn
            .query_row(
                &format!(r#"SELECT COUNT(*) FROM "CoexistSyncRun" WHERE "workspaceId" = ?1 AND "type" = 'contact_scan' AND (?2 = ?2){filter}"#),
                [workspace_id, integration],
                |r| r.get(0),
            )
            .map_err(repo_err)?;
        let mut stmt = conn
            .prepare(&format!(
                r#"SELECT {COLUMNS} FROM "CoexistSyncRun"
                   WHERE "workspaceId" = ?1 AND "type" = 'contact_scan' AND (?2 = ?2){filter}
                   ORDER BY "createdAt" DESC, "id" DESC LIMIT ?3 OFFSET ?4"#
            ))
            .map_err(repo_err)?;
        let data = stmt
            .query_map([workspace_id, integration, per_page, (page - 1) * per_page], run)
            .map_err(repo_err)?
            .map(|r| r.map(|run| history_item(&run)))
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(repo_err)?;
        Ok(json!({ "data": data, "pageCount": (total + per_page - 1) / per_page }))
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(status: &str, created_at: i64) -> Run {
        Run {
            id: 1,
            channel: "messenger".into(),
            status: status.into(),
            scan_from_at: None,
            imported: 0,
            current_scan: 0,
            started_at: None,
            finished_at: None,
            created_at,
            requested_by: None,
            current_error: None,
        }
    }

    #[test]
    fn availability_rules() {
        assert_eq!(availability(None, 0), json!({ "canScan": true }));
        let running = availability(Some(&at("running", 0)), 1);
        assert_eq!(running["blockedReason"], "running");
        assert_eq!(running["nextScanAt"], iso(COOLDOWN_MS));
        assert_eq!(
            availability(Some(&at("succeeded", 0)), 1)["blockedReason"],
            "cooldown"
        );
        assert_eq!(
            availability(Some(&at("failed", 0)), COOLDOWN_MS),
            json!({ "canScan": true })
        );
    }
}
