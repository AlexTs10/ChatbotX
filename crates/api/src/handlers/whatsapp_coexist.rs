//! `createCoexistRoute(channel)` of `features/channel-integrations/api/public.ts`
//! (scope `channels`): `GET` / `PUT /v1/{whatsapp,messenger,instagram}-channels/{id}/coexist`.
use chatbotx_business::BusinessError;
use chatbotx_business::coexist::{self, SetCoexist};
use serde_json::{Value, json};

use super::input::{Input, key};
use super::tags::digits_at;
use super::{ApiState, Call, ErrorMap, FINDING, MUTATING, Reply, business_error, json};

/// The routes of this module.
pub const ROUTES: &[&str] = &[
    "whatsappChannels.getCoexistStatus",
    "whatsappChannels.setCoexist",
    "messengerChannels.getCoexistStatus",
    "messengerChannels.setCoexist",
    "instagramChannels.getCoexistStatus",
    "instagramChannels.setCoexist",
];

/// `possibleErrorsOnSettingCoexist`.
const SETTING: ErrorMap = &[
    ("notFound", 404),
    ("businessError", 400),
    ("idempotencyKeyInvalid", 422),
    ("idempotencyKeyReused", 422),
    ("idempotencyKeyConflict", 409),
    ("coexistInvalidAuth", 409),
    ("coexistSyncNotStarted", 502),
];

fn not_found() -> BusinessError {
    BusinessError::not_found("Channel not found")
}

pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    let ws = call.workspace_id;
    let channel = match route_id.split_once("Channels.") {
        Some(("messenger", _)) => "messenger",
        Some(("instagram", _)) => "instagram",
        _ => "whatsapp",
    };
    if route_id.ends_with(".getCoexistStatus") {
        let mut input = Input::from_query(call.params, call.query);
        let value = input.get("id").cloned();
        let id = digits_at(&mut input, &key("id"), value.as_ref(), true).unwrap_or_default();
        input.finish()?;
        return coexist::coexist_status(&state.db, channel, ws, id)
            .await
            .and_then(|status| status.ok_or_else(not_found))
            .map(|status| json(200, status))
            .map_err(|e| business_error(e, FINDING));
    }
    let mut input = Input::from_body(call.params, call.body);
    let value = input.get("id").cloned();
    let id = digits_at(&mut input, &key("id"), value.as_ref(), true).unwrap_or_default();
    let enabled = input.bool("enabled");
    let ai_reads = input.opt_bool("aiReadsSyncedHistory");
    input.finish()?;
    if channel != "whatsapp" {
        return match coexist::set_pull_coexist(
            &state.db,
            channel,
            ws,
            id,
            enabled.unwrap_or_default(),
            ai_reads.unwrap_or(false),
        )
        .await
        {
            Ok(Some(run)) => Ok(json(200, pull_output(run))),
            Ok(None) => Err(business_error(not_found(), SETTING)),
            Err(error) => Err(business_error(error, MUTATING)),
        };
    }
    let result = coexist::set_whatsapp_coexist(
        &state.db,
        &state.graph_url,
        ws,
        id,
        enabled.unwrap_or_default(),
        ai_reads.unwrap_or(false),
    )
    .await
    .map_err(|e| business_error(e, MUTATING))?;
    let sync_not_started = |reason: &str| {
        BusinessError::exception(
            format!("Coexist is on, but Meta did not start the sync ({reason}). Try again."),
            "coexistSyncNotStarted",
            502,
        )
    };
    let error = match result {
        SetCoexist::Done => return Ok(json(200, json!({ "success": true }))),
        SetCoexist::NotFound => not_found(),
        SetCoexist::InvalidAuth => BusinessError::exception(
            "The channel's credentials are invalid: reconnect the channel.",
            "coexistInvalidAuth",
            409,
        ),
        SetCoexist::TriggerRejected(reason) => {
            sync_not_started(reason.as_deref().unwrap_or("triggerRejected"))
        }
        SetCoexist::TriggerThrew(_) => sync_not_started("trigger_failed"),
    };
    Err(business_error(error, SETTING))
}

/// The `setCoexist` / `setCoexist{Messenger,Instagram}API` success of a pull
/// channel (`runId` only when enabling).
pub fn pull_output(run: Option<i64>) -> Value {
    match run {
        Some(id) => json!({ "success": true, "runId": id.to_string() }),
        None => json!({ "success": true }),
    }
}

/// `integrationWhatsappAPIs.setCoexistWhatsappAPI` output
/// (`setCoexistResponseSchema`: `cause` is stripped).
pub fn rpc_output(result: &SetCoexist) -> Value {
    match result {
        SetCoexist::Done => json!({ "success": true }),
        SetCoexist::NotFound | SetCoexist::InvalidAuth | SetCoexist::TriggerRejected(None) => {
            json!({ "success": false })
        }
        SetCoexist::TriggerRejected(Some(reason)) => json!({ "success": false, "reason": reason }),
        SetCoexist::TriggerThrew(msg) => {
            let mut out = json!({ "success": false, "reason": "trigger_failed" });
            if let Some(msg) = msg {
                out["msg"] = Value::String(msg.clone());
            }
            out
        }
    }
}
