//! `features/response-sla/api/public.ts` (`responseSlaPublicRouter`, scope
//! `inbox`), plus the request check the `/rpc` action shares.

use chatbotx_business::response_sla::{self as service, MAX_TARGET_SECONDS, MIN_TARGET_SECONDS};
use chatbotx_db::repo::response_sla::ResponseSlaPolicyInput;
use serde_json::Value;

use super::input::{Input, Num};
use super::{ApiState, Call, FINDING, MUTATING, Reply, business_error, json, no_content};

/// The routes of this module.
pub const ROUTES: &[&str] = &["responseSla.get", "responseSla.update", "responseSla.delete"];

const TARGET: Num = Num::int(MIN_TARGET_SECONDS as f64, MAX_TARGET_SECONDS as f64);

/// `updateResponseSlaRequest`.
pub fn request(input: &mut Input) -> ResponseSlaPolicyInput {
    let enabled = input.opt_bool("enabled").unwrap_or(true);
    let first = input.number("firstResponseTargetSeconds", TARGET);
    // `.nullable()`, not optional: absent is an issue.
    let next = match input.get("nextResponseTargetSeconds") {
        Some(Value::Null) => None,
        _ => input.number("nextResponseTargetSeconds", TARGET),
    };
    let percent = input.opt_number("breachingThresholdPercent", Num::int(1.0, 99.0));
    ResponseSlaPolicyInput {
        enabled,
        first_response_target_seconds: first.unwrap_or_default() as i64,
        next_response_target_seconds: next.map(|n| n as i64),
        breaching_threshold_percent: percent.map_or(25, |n| n as i64),
    }
}

/// `upsertPolicy` + its audit.
pub async fn upsert(
    state: &ApiState,
    workspace_id: i64,
    data: ResponseSlaPolicyInput,
) -> chatbotx_business::Result<Value> {
    let (row, detail) = service::upsert_policy(&state.db, workspace_id, data).await?;
    state.record_audit("update", detail).await;
    Ok(service::resource(&row))
}

/// `{ enabled }` of the builder status procedure.
pub async fn enabled(state: &ApiState, workspace_id: i64) -> chatbotx_business::Result<bool> {
    Ok(service::find_policy(&state.db, workspace_id)
        .await?
        .is_some_and(|p| p.enabled))
}

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    let ws = call.workspace_id;
    match route_id {
        "responseSla.get" => service::find_policy(&state.db, ws)
            .await
            .map(|p| json(200, p.as_ref().map_or(Value::Null, service::resource)))
            .map_err(|e| business_error(e, FINDING)),
        "responseSla.update" => {
            let mut input = Input::from_body(call.params, call.body);
            let data = request(&mut input);
            input.finish()?;
            upsert(state, ws, data)
                .await
                .map(|out| json(200, out))
                .map_err(|e| business_error(e, MUTATING))
        }
        "responseSla.delete" => {
            let deleted = service::delete_policy(&state.db, ws)
                .await
                .map_err(|e| business_error(e, MUTATING))?;
            if deleted {
                state
                    .record_audit("delete", "removed the response SLA policy")
                    .await;
            }
            Ok(no_content())
        }
        _ => Err(crate::routes::not_ported_error()),
    }
}
