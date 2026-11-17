//! Q.D9 slice 7: the public booking webview. The data of the server pages
//! `app/booking/{picker,range-picker,schedule,cancel}/page.tsx` as page
//! loaders (`POST /rpc/pages/booking*`, `{ json: { token } }`) and the three
//! public actions `submitBookingAction`, `submitAvailabilityRangeAction`,
//! `cancelBookingAction` (`app/booking/*/action*.ts`, `actionClient`: no
//! session; the signed token is the authority).
//!
//! Every call is limited per client IP with the guest limiter (the original
//! has no limit on these routes). A bad or expired token: the loaders answer
//! `null` (the page shows "invalid link"), the actions the default server
//! error. The answers are JSON for the SPA (React escapes the text); nothing
//! is rendered as HTML here.

use std::sync::Arc;

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use chatbotx_business::BusinessError;
use chatbotx_business::appointment::booking;
use chatbotx_business::appointment::calendar;
use chatbotx_business::contact_custom_field::{self, SetValueByKeyInput};
use chatbotx_business::tx;
use chatbotx_core::encryption::{
    AppointmentClaims, AppointmentWebviewClaims, AppointmentWebviewMode,
};
use chatbotx_db::repo::{iso, now_ms};
use chatbotx_jobs::{Enqueuer as _, JobOptions, QueueName};
use serde_json::{Value, json};

use crate::handlers::ApiState;
use crate::handlers::appointments::{deps, externals};
use crate::rpc::rpc_error;

/// `DEFAULT_SLOT_WINDOW_DAYS`.
const SLOT_WINDOW_MS: i64 = 90 * 86_400_000;
const DEFAULT_SERVER_ERROR: &str = "Something went wrong while executing the operation.";
const WEBVIEW_SELECTION: &str = "appointmentWebviewSelection";
const RANGE_SELECTION: &str = "appointmentAvailabilityRangeSelection";
const RANGE_SKIPPED: &str = "appointmentAvailabilityRangeSkipped";

/// The routes (merged before the `/rpc/{*rest}` router: static paths win).
pub fn router(state: Arc<ApiState>) -> Router {
    Router::new()
        .route("/rpc/pages/bookingPicker", post(picker_page))
        .route("/rpc/pages/bookingRangePicker", post(range_page))
        .route("/rpc/pages/bookingSchedule", post(schedule_page))
        .route("/rpc/pages/bookingCancel", post(cancel_page))
        .route("/rpc/actions/submitBookingAction", post(submit_booking))
        .route(
            "/rpc/actions/submitAvailabilityRangeAction",
            post(submit_range),
        )
        .route("/rpc/actions/cancelBookingAction", post(cancel_booking))
        .with_state(state)
}

fn ok(value: Value) -> Response {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        json!({ "json": value, "meta": [] }).to_string(),
    )
        .into_response()
}

fn server_error(message: &str) -> Response {
    rpc_error(StatusCode::BAD_REQUEST, "BAD_REQUEST", message)
}

fn invalid_input() -> Response {
    rpc_error(
        StatusCode::BAD_REQUEST,
        "BAD_REQUEST",
        "Input validation failed",
    )
}

/// `{ json: <input> }`; an action's input is `args[0]`.
fn body_json(body: &Bytes) -> Value {
    serde_json::from_slice::<Value>(body)
        .ok()
        .and_then(|payload| payload.get("json").cloned())
        .unwrap_or(Value::Null)
}

fn action_input(body: &Bytes) -> Value {
    body_json(body)["args"][0].clone()
}

/// The guest limiter: `None` = allowed.
fn limited(state: &ApiState, headers: &HeaderMap) -> Option<Response> {
    let ip = crate::auth::guest_client_ip(headers);
    let limit =
        crate::reflink_widget::check_guest_rate_limit(state, "booking", &ip, None, now_ms());
    limit.limited.then(|| {
        let mut response = rpc_error(
            StatusCode::TOO_MANY_REQUESTS,
            "TOO_MANY_REQUESTS",
            "Too many requests",
        );
        if let Ok(value) = limit.retry_after.to_string().parse() {
            response.headers_mut().insert(header::RETRY_AFTER, value);
        }
        response
    })
}

fn token_of(input: &Value) -> Option<&str> {
    input["token"].as_str().filter(|t| !t.is_empty())
}

fn webview_claims(state: &ApiState, token: &str) -> Option<AppointmentWebviewClaims> {
    state
        .encryption
        .as_deref()?
        .verify_appointment_webview_token(token)
        .ok()
        .map(|payload| payload.claims)
}

fn ids(claims: &AppointmentWebviewClaims) -> Option<(i64, i64, i64)> {
    Some((
        claims.workspace_id.parse().ok()?,
        claims.calendar_id.parse().ok()?,
        claims.contact_id.parse().ok()?,
    ))
}

fn parse_ms(text: Option<&str>) -> Option<i64> {
    text.and_then(chatbotx_core::js::parse_date)
}

// ---- page loaders -----------------------------------------------------------

/// `booking/picker/page.tsx`.
async fn picker_page(
    State(state): State<Arc<ApiState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Some(response) = limited(&state, &headers) {
        return response;
    }
    let input = body_json(&body);
    let Some(claims) = token_of(&input).and_then(|t| webview_claims(&state, t)) else {
        return ok(Value::Null);
    };
    if claims.mode == AppointmentWebviewMode::SelectAvailabilityRange {
        return ok(Value::Null);
    }
    let Some((ws, calendar_id, contact_id)) = ids(&claims) else {
        return ok(Value::Null);
    };
    let Ok(found) = tx::read(&state.db, move |conn| {
        calendar::find_or_fail(conn, ws, calendar_id)
    })
    .await
    else {
        return ok(Value::Null);
    };
    let start = parse_ms(claims.availability_start_at.as_deref()).unwrap_or_else(now_ms);
    let end = parse_ms(claims.availability_end_at.as_deref()).unwrap_or(start + SLOT_WINDOW_MS);
    let external = externals(&state);
    let Ok(slots) = calendar::resolve_available_slots_for_listing(
        &state.db,
        &external,
        ws,
        calendar_id,
        Some(contact_id),
        start,
        end,
    )
    .await
    else {
        return ok(Value::Null);
    };
    ok(json!({
        "calendarName": found.name,
        "description": found.description,
        "timezone": found.timezone,
        "closeOnSuccess": claims.node_id.is_some(),
        "submitMode": if claims.mode == AppointmentWebviewMode::SelectAvailability { "select" } else { "book" },
        "slots": slots.iter().map(|s| json!({ "startAt": iso(s.start_at), "endAt": iso(s.end_at) })).collect::<Vec<_>>(),
    }))
}

/// `booking/range-picker/page.tsx`.
async fn range_page(
    State(state): State<Arc<ApiState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Some(response) = limited(&state, &headers) {
        return response;
    }
    let input = body_json(&body);
    let Some(claims) = token_of(&input).and_then(|t| webview_claims(&state, t)) else {
        return ok(Value::Null);
    };
    let Some((ws, calendar_id, _)) =
        ids(&claims).filter(|_| claims.mode == AppointmentWebviewMode::SelectAvailabilityRange)
    else {
        return ok(Value::Null);
    };
    match tx::read(&state.db, move |conn| {
        calendar::find_or_fail(conn, ws, calendar_id)
    })
    .await
    {
        Ok(found) => ok(json!({
            "calendarName": found.name,
            "description": found.description,
            "timezone": found.timezone,
            "closeOnSuccess": claims.node_id.is_some(),
        })),
        Err(_) => ok(Value::Null),
    }
}

async fn detail(state: &ApiState, claims: &AppointmentClaims) -> Option<Value> {
    let ws = claims.workspace_id.parse().ok()?;
    let id = claims.appointment_id.parse().ok()?;
    let contact = claims.contact_id.parse().ok()?;
    booking::schedule_detail(&state.db, ws, id, contact)
        .await
        .ok()
}

/// `booking/schedule/page.tsx`: the detail and a fresh cancel token.
async fn schedule_page(
    State(state): State<Arc<ApiState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Some(response) = limited(&state, &headers) {
        return response;
    }
    let input = body_json(&body);
    let Some(encryption) = state.encryption.as_deref() else {
        return ok(Value::Null);
    };
    let Some(payload) =
        token_of(&input).and_then(|t| encryption.verify_appointment_schedule_token(t).ok())
    else {
        return ok(Value::Null);
    };
    let Some(mut found) = detail(&state, &payload.claims).await else {
        return ok(Value::Null);
    };
    let claims = AppointmentClaims {
        appointment_id: found["id"].as_str().unwrap_or_default().to_string(),
        workspace_id: found["workspaceId"]
            .as_str()
            .unwrap_or_default()
            .to_string(),
        contact_id: found["contactId"].as_str().unwrap_or_default().to_string(),
        conversation_id: found["conversationId"].as_str().map(str::to_string),
        contact_inbox_id: payload.claims.contact_inbox_id.clone(),
        flow_version_id: payload.claims.flow_version_id.clone(),
    };
    let Ok(cancel) = encryption.sign_appointment_cancel_token(
        &claims,
        chatbotx_core::encryption::APPOINTMENT_TOKEN_TTL_MS,
    ) else {
        return ok(Value::Null);
    };
    found["cancelToken"] = json!(cancel);
    ok(found)
}

/// `booking/cancel/page.tsx`.
async fn cancel_page(
    State(state): State<Arc<ApiState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Some(response) = limited(&state, &headers) {
        return response;
    }
    let input = body_json(&body);
    let Some(payload) = token_of(&input).and_then(|t| {
        state
            .encryption
            .as_deref()?
            .verify_appointment_cancel_token(t)
            .ok()
    }) else {
        return ok(Value::Null);
    };
    ok(detail(&state, &payload.claims).await.unwrap_or(Value::Null))
}

// ---- actions ----------------------------------------------------------------

fn not_completed(stale: bool) -> Value {
    json!({
        "staleSlot": stale,
        "availabilityChanged": !stale,
        "completed": false,
        "appointment": null,
        "scheduleUrl": null,
        "cancelUrl": null,
    })
}

/// `submitBookingAction`.
async fn submit_booking(
    State(state): State<Arc<ApiState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Some(response) = limited(&state, &headers) {
        return response;
    }
    let input = action_input(&body);
    let selected_text = input["selectedStartAt"].as_str().unwrap_or_default();
    let timezone = input["inviteeTimezone"].as_str().unwrap_or_default().trim();
    let selected =
        chatbotx_core::js::parse_date(selected_text).filter(|_| selected_text.contains('T'));
    let (Some(token), Some(selected)) = (token_of(&input), selected) else {
        return invalid_input();
    };
    if timezone.is_empty() || timezone.chars().count() > 64 {
        return invalid_input();
    }
    let Some(claims) = webview_claims(&state, token) else {
        return server_error(DEFAULT_SERVER_ERROR);
    };
    let Some((ws, calendar_id, contact_id)) = ids(&claims) else {
        return server_error(DEFAULT_SERVER_ERROR);
    };
    let external = externals(&state);
    let deps = deps(&state, &external);
    let range_start = parse_ms(claims.availability_start_at.as_deref());
    let range_end = parse_ms(claims.availability_end_at.as_deref());
    let outcome: Result<Value, BusinessError> = match claims.mode {
        AppointmentWebviewMode::Book => {
            let range = range_start.zip(range_end);
            booking::complete_webview_booking(
                deps,
                &claims,
                range,
                selected,
                timezone,
                &state.app_url,
            )
            .await
            .map(|(appointment, schedule, cancel)| {
                json!({
                    "appointment": { "id": appointment.id.to_string() },
                    "scheduleUrl": schedule,
                    "cancelUrl": cancel,
                })
            })
        }
        AppointmentWebviewMode::SelectAvailability => {
            calendar::resolve_available_slots_for_listing(
                &state.db,
                &external,
                ws,
                calendar_id,
                Some(contact_id),
                range_start.unwrap_or(selected),
                range_end.unwrap_or(selected),
            )
            .await
            .and_then(|slots| {
                if slots.iter().any(|s| s.start_at == selected) {
                    Ok(json!({ "appointment": null, "scheduleUrl": null, "cancelUrl": null }))
                } else {
                    Err(booking::slot_unavailable())
                }
            })
        }
        AppointmentWebviewMode::SelectAvailabilityRange => {
            return server_error(DEFAULT_SERVER_ERROR);
        }
    };
    let mut result = match outcome {
        Ok(result) => result,
        Err(error) => {
            return match error.code() {
                Some("slotUnavailable" | "appointmentAlreadyScheduled") => ok(not_completed(true)),
                Some("appointmentAvailabilityChanged") => ok(not_completed(false)),
                _ => match error {
                    BusinessError::Exception {
                        message, status, ..
                    } if status < 500 => server_error(&message),
                    _ => {
                        tracing::error!(%error, "submitBookingAction failed");
                        server_error(DEFAULT_SERVER_ERROR)
                    }
                },
            };
        }
    };
    if let Some(field) = claims.selected_date_custom_field_id.clone() {
        let write = SetValueByKeyInput {
            workspace_id: ws,
            contact_id,
            keyword: field,
            value: selected_text.to_string(),
            ..SetValueByKeyInput::default()
        };
        if let Err(error) = contact_custom_field::set_value_by_key(&state.db, write).await {
            tracing::warn!(%error, workspace_id = ws, reason = "setSelectedDateCustomFieldFailed", "Failed to persist selected appointment date custom field");
        }
    }
    let appointment_id = result["appointment"]["id"].clone();
    if let (Some(node), Some(flow), Some(version), Some(step)) = (
        &claims.node_id,
        &claims.flow_id,
        &claims.flow_version_id,
        &claims.step_id,
    ) {
        let data = json!({
            "conversationId": claims.conversation_id,
            "contactInboxId": claims.contact_inbox_id,
            "flowId": flow,
            "flowVersionId": version,
            "nodeId": node,
            "startFromStepId": step,
            "appointmentId": appointment_id,
            "metadata": {
                "type": WEBVIEW_SELECTION,
                "stepId": step,
                "contactInboxId": claims.contact_inbox_id,
                "selectedStartAt": selected_text,
                "appointmentId": appointment_id,
            },
            "origin": "channel",
        });
        if let Err(error) = send_flow(&state, data, JobOptions::default()).await {
            tracing::warn!(%error, workspace_id = ws, reason = "resumeFlowAfterBookingFailed", "Failed to resume flow after appointment booking");
        }
    }
    result["staleSlot"] = json!(false);
    result["completed"] = json!(true);
    ok(result)
}

async fn send_flow(state: &ApiState, data: Value, options: JobOptions) -> Result<(), String> {
    let Some(jobs) = state.jobs.as_deref() else {
        return Err("no job queue".into());
    };
    jobs.add(
        QueueName::Integration,
        "sendFlow",
        json!({ "type": "sendFlow", "data": data }),
        options,
    )
    .await
    .map(|_| ())
    .map_err(|e| e.to_string())
}

/// `z.iso.datetime({ local: true })`: `YYYY-MM-DDTHH:mm[:ss[.fff]]`, no zone.
fn local_datetime(value: &Value) -> Option<&str> {
    let text = value.as_str()?;
    let ok = text.len() >= 16
        && text.as_bytes().get(10) == Some(&b'T')
        && !text.ends_with('Z')
        && !text[10..].contains(['+', '-'])
        && chatbotx_core::js::parse_date(text).is_some();
    ok.then_some(text)
}

/// `submitAvailabilityRangeAction`.
async fn submit_range(
    State(state): State<Arc<ApiState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Some(response) = limited(&state, &headers) {
        return response;
    }
    let input = action_input(&body);
    let Some(token) = token_of(&input) else {
        return invalid_input();
    };
    let skip = input["skip"] == json!(true);
    let range = if skip {
        None
    } else {
        match (
            local_datetime(&input["startDate"]),
            local_datetime(&input["endDate"]),
        ) {
            (Some(start), Some(end)) if start <= end => Some((start.to_string(), end.to_string())),
            _ => return invalid_input(),
        }
    };
    let Some(claims) = webview_claims(&state, token)
        .filter(|c| c.mode == AppointmentWebviewMode::SelectAvailabilityRange)
    else {
        return server_error(DEFAULT_SERVER_ERROR);
    };
    let (Some(flow), Some(version), Some(step)) =
        (&claims.flow_id, &claims.flow_version_id, &claims.step_id)
    else {
        return server_error(DEFAULT_SERVER_ERROR);
    };
    let mut data = json!({
        "conversationId": claims.conversation_id,
        "contactInboxId": claims.contact_inbox_id,
        "flowId": flow,
        "flowVersionId": version,
        "nodeId": claims.node_id,
        "startFromStepId": step,
        "origin": "channel",
    });
    let job_id = match &range {
        None => {
            data["metadata"] = json!({ "type": RANGE_SKIPPED, "stepId": step, "contactInboxId": claims.contact_inbox_id });
            format!("appointment-range-skip-{}-{step}", claims.conversation_id)
        }
        Some((start, end)) => {
            data["metadata"] = json!({
                "type": RANGE_SELECTION,
                "stepId": step,
                "contactInboxId": claims.contact_inbox_id,
                "startDate": start,
                "endDate": end,
            });
            format!(
                "appointment-range-{}-{step}-{}-{}",
                claims.conversation_id,
                chatbotx_core::js::parse_date(start).unwrap_or_default(),
                chatbotx_core::js::parse_date(end).unwrap_or_default()
            )
        }
    };
    let options = JobOptions {
        job_id: Some(job_id),
        ..JobOptions::default()
    };
    match send_flow(&state, data, options).await {
        Ok(()) => ok(json!({ "completed": true })),
        Err(error) => {
            tracing::warn!(%error, reason = "resumeFlowAfterAvailabilityRangeFailed", "Failed to resume flow after appointment availability range selection");
            server_error(DEFAULT_SERVER_ERROR)
        }
    }
}

/// `cancelBookingAction`: `{ cancellable, appointment }`.
async fn cancel_booking(
    State(state): State<Arc<ApiState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Some(response) = limited(&state, &headers) {
        return response;
    }
    let input = action_input(&body);
    let Some(token) = token_of(&input) else {
        return invalid_input();
    };
    let Some(payload) = state
        .encryption
        .as_deref()
        .and_then(|e| e.verify_appointment_cancel_token(token).ok())
    else {
        return server_error(DEFAULT_SERVER_ERROR);
    };
    let claims = payload.claims;
    let parsed = (
        claims.workspace_id.parse::<i64>(),
        claims.appointment_id.parse::<i64>(),
        claims.contact_id.parse::<i64>(),
    );
    let (Ok(ws), Ok(id), Ok(contact)) = parsed else {
        return server_error(DEFAULT_SERVER_ERROR);
    };
    let inbox = claims
        .contact_inbox_id
        .as_deref()
        .and_then(|i| i.parse().ok());
    let external = externals(&state);
    match booking::cancel_by_token(deps(&state, &external), ws, id, contact, inbox).await {
        Ok((cancellable, appointment)) => ok(json!({
            "cancellable": cancellable,
            "appointment": crate::handlers::appointments::appointment_json(&appointment),
        })),
        Err(BusinessError::Exception {
            message, status, ..
        }) if status < 500 => server_error(&message),
        Err(error) => {
            tracing::error!(%error, "cancelBookingAction failed");
            server_error(DEFAULT_SERVER_ERROR)
        }
    }
}
