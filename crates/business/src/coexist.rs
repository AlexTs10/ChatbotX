//! `coexistService.enable` / `disable` and `integration-whatsapp/coexist.ts`
//! `setCoexist` with the `triggerSmbAppDataSync` provider call
//! (`integrations/whatsapp/src/api/coexists.ts`).
use std::time::Duration;

use chatbotx_db::repo::coexist_sync_run::{self as runs, CoexistIntegrationRow, CreateRunInput};
use serde_json::{Value, json};

use crate::{Result, tx};

/// The two sync types, in order (`WHATSAPP_COEXIST_SYNC_TYPES`).
pub const WHATSAPP_COEXIST_SYNC_TYPES: [&str; 2] = ["smb_app_state_sync", "history"];

/// `SetCoexistResult` (`cause` + `reason` / `msg`).
#[derive(Debug, Clone, PartialEq)]
pub enum SetCoexist {
    Done,
    NotFound,
    InvalidAuth,
    /// Meta refused a sync type (`already_triggered`, `window_expired`,
    /// `not_eligible`), or accepted nothing without a reason.
    TriggerRejected(Option<String>),
    /// The call failed (`reason: "trigger_failed"`); the provider message.
    TriggerThrew(Option<String>),
}

/// `triggerSmbAppDataSync`: `Ok(None)` = accepted, `Ok(Some(reason))` = a
/// known refusal, `Err(message)` = anything else.
pub async fn trigger_smb_app_data_sync(
    graph_url: &str,
    token: &str,
    version: &str,
    phone_number_id: &str,
    sync_type: &str,
) -> std::result::Result<Option<&'static str>, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .unwrap_or_default();
    let response = client
        .post(format!(
            "{graph_url}/{version}/{phone_number_id}/smb_app_data"
        ))
        .bearer_auth(token)
        .json(&json!({ "messaging_product": "whatsapp", "sync_type": sync_type }))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let ok = response.status().is_success();
    let body = response.text().await.map_err(|e| e.to_string())?;
    if ok {
        if body.trim().is_empty() {
            return Ok(None);
        }
        let parsed: Value = serde_json::from_str(&body)
            .map_err(|_| "smb_app_data returned a non-JSON response".to_owned())?;
        let accepted = match (parsed.get("success"), parsed.get("request_id")) {
            (Some(Value::Bool(success)), _) => *success,
            (_, Some(Value::String(id))) if !id.is_empty() => true,
            _ => return Err("smb_app_data returned an unexpected response".into()),
        };
        return if accepted {
            Ok(None)
        } else {
            Err(format!("smb_app_data {sync_type} failed"))
        };
    }
    let error = serde_json::from_str::<Value>(&body).unwrap_or(Value::Null)["error"].clone();
    let message = error["message"].as_str().unwrap_or_default().to_owned();
    if error["code"].as_i64() == Some(131_000) && error["error_subcode"].as_i64() == Some(10) {
        tracing::warn!(
            sync_type,
            phone_number_id,
            "smb_app_data not eligible (non-SMB phone)"
        );
        return Ok(Some("not_eligible"));
    }
    let lower = message.to_lowercase();
    if lower.contains("already") && lower.contains("trigger") {
        return Ok(Some("already_triggered"));
    }
    if lower.contains("window") || lower.contains("expired") || lower.contains("24 hour") {
        return Ok(Some("window_expired"));
    }
    tracing::error!(code = ?error["code"], %message, "smb_app_data unknown error");
    Err(if message.is_empty() {
        "smb_app_data error".into()
    } else {
        message
    })
}

/// `setCoexist` for a WhatsApp number: disable (fail the live runs) or
/// enable (create or reuse the live run) and ask Meta for both syncs.
pub async fn set_whatsapp_coexist(
    db: &chatbotx_db::Db,
    graph_url: &str,
    workspace_id: i64,
    integration_id: i64,
    enabled: bool,
    ai_reads_synced_history: bool,
) -> Result<SetCoexist> {
    let (ws, id) = (workspace_id, integration_id);
    if !enabled {
        return tx::write(db, move |c| {
            if runs::set_integration_coexist_enabled(c, "whatsapp", ws, id, false, None)?.is_none()
            {
                return Ok(SetCoexist::NotFound);
            }
            runs::tear_down_active_runs_for_integration(c, "whatsapp", id, "Coexist disabled")?;
            Ok(SetCoexist::Done)
        })
        .await;
    }
    let row = tx::read(db, move |c| {
        Ok(runs::find_integration_for_coexist(c, ws, id, "whatsapp")?)
    })
    .await?;
    let Some(CoexistIntegrationRow::Whatsapp(integration)) = row else {
        return Ok(SetCoexist::NotFound);
    };
    let Some(token) = integration
        .auth
        .0
        .pointer("/tokens/accessToken")
        .and_then(Value::as_str)
    else {
        return Ok(SetCoexist::InvalidAuth);
    };
    let version = integration
        .auth
        .0
        .get("version")
        .and_then(Value::as_str)
        .unwrap_or(crate::whatsapp::DEFAULT_API_VERSION);
    let run_id = tx::write(db, move |c| {
        if runs::find_integration_for_coexist(c, ws, id, "whatsapp")?.is_none() {
            return Ok(None);
        }
        runs::set_integration_coexist_enabled(
            c,
            "whatsapp",
            ws,
            id,
            true,
            Some(ai_reads_synced_history),
        )?;
        if let Some(live) = runs::find_live_run(c, id, "whatsapp")? {
            return Ok(Some(live.id));
        }
        let input = CreateRunInput {
            workspace_id: ws,
            integration_id: id,
            channel: "whatsapp",
            trigger_source: "popup-enable",
        };
        Ok(Some(runs::create_run(c, &input)?.id))
    })
    .await?;
    let Some(run_id) = run_id else {
        return Ok(SetCoexist::NotFound);
    };
    let fail_run = |error: String| async move {
        tx::write(db, move |c| Ok(runs::mark_failed(c, run_id, &error, None)?)).await
    };
    for sync_type in WHATSAPP_COEXIST_SYNC_TYPES {
        match trigger_smb_app_data_sync(
            graph_url,
            token,
            version,
            &integration.phone_number_id,
            sync_type,
        )
        .await
        {
            Ok(None) => {}
            Ok(Some(reason)) => {
                fail_run(format!("smb_app_data {reason}")).await?;
                return Ok(SetCoexist::TriggerRejected(Some(reason.to_owned())));
            }
            Err(message) => {
                tracing::error!(
                    error = %message,
                    integration_id,
                    "smb_app_data trigger failed"
                );
                fail_run(message.clone()).await?;
                return Ok(SetCoexist::TriggerThrew(Some(message)));
            }
        }
    }
    Ok(SetCoexist::Done)
}

/// `coexistRunResource` of `getCoexistStatus`: `None` = no such channel.
pub async fn whatsapp_coexist_status(
    db: &chatbotx_db::Db,
    workspace_id: i64,
    integration_id: i64,
) -> Result<Option<Value>> {
    let (ws, id) = (workspace_id, integration_id);
    tx::read(db, move |c| {
        if runs::find_integration_for_coexist(c, ws, id, "whatsapp")?.is_none() {
            return Ok(None);
        }
        let run = runs::find_latest_run(c, ws, id, "whatsapp")?;
        let date = |ms: Option<i64>| {
            ms.map_or(Value::Null, |ms| Value::String(chatbotx_db::repo::iso(ms)))
        };
        Ok(Some(json!({ "run": run.map(|r| json!({
            "id": r.id.to_string(),
            "status": r.status,
            "startedAt": date(r.started_at),
            "finishedAt": date(r.finished_at),
            "totalScan": r.total_scan,
            "currentScan": r.current_scan,
            "currentStep": r.current_step,
            "syncProgress": r.sync_progress,
            "importedContactCount": r.imported_contact_count,
            "importedMessageCount": r.imported_message_count,
            "skippedCount": r.skipped_count,
            "failedCount": r.failed_count,
        })) })))
    })
    .await
}
