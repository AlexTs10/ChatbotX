//! `apps/builder/src/features/integration-ai-handover/api/public.ts`
//! (`aiHandoverPublicRouter`), its request schemas (`schema/public.ts`) and
//! the resource mappers (`lib/bulk-run-resource.ts`).

use chatbotx_business::ai_handover::{
    self as service, ApplyToAllStatus, SaveSettingsInput, SetApplyToAllInput, SettingsPatch, codes,
};
use chatbotx_business::tx;
use chatbotx_db::repo::ai_handover_bulk_run::{AI_HANDOVER_BULK_LIVE_STATUSES, RequestedBy};
use chatbotx_db::repo::models::{AiHandoverBulkRun, AiHandoverSettings};
use chatbotx_db::repo::now_ms;
use chatbotx_db::repo::utils::PaginationInput;
use serde_json::{Value, json};

use super::input::{Input, Num, Seg, Str, child, key};
use super::qr_codes::public_paging;
use super::{ApiState, Call, ErrorMap, FINDING, Reply, business_error, date, id, json};

/// The ported routes.
pub const ROUTES: &[&str] = &[
    "aiHandover.getSettings",
    "aiHandover.saveSettings",
    "aiHandover.patchSettings",
    "aiHandover.setApplyToAll",
    "aiHandover.retryApplyToAll",
    "aiHandover.listHistory",
];

/// `AI_HANDOVER_MESSAGE_MAX_LENGTH` (the loosest channel limit).
pub const MESSAGE_MAX_LENGTH: usize = 2000;

/// `possibleErrorsOnApplyingAiHandover`.
pub const APPLYING: ErrorMap = &[
    ("notFound", 404),
    ("businessError", 400),
    (codes::CONFIRM_COUNT_EXCEEDED, 422),
    (codes::RUN_NOT_STARTABLE, 422),
    (codes::PAGE_NOT_CONNECTED, 422),
    (codes::AUTOMATION_NOT_ACTIVE, 422),
    (codes::NOTHING_TO_RETRY, 422),
    (codes::MESSAGE_REQUIRED, 422),
    (codes::MESSAGE_TOO_LONG, 422),
    ("idempotencyKeyInvalid", 422),
    ("idempotencyKeyReused", 422),
    ("idempotencyKeyConflict", 409),
];

/// `toSettingsResource` (defaults for a Page that never saved).
pub fn settings_json(s: Option<&AiHandoverSettings>) -> Value {
    json!({
        "enabled": s.is_some_and(|s| s.enabled),
        "scheduleEnabled": s.is_some_and(|s| s.schedule_enabled),
        "timeRanges": s.map_or(json!([]), |s| s.time_ranges.0.clone()),
        "gotoFlowId": s.and_then(|s| s.goto_flow_id).map(id),
        "returnMessage": s.and_then(|s| s.return_message.clone()),
        "pauseBotWaitingForStaff": s.is_some_and(|s| s.pause_bot_waiting_for_staff),
    })
}

/// `toBulkRunResource`: `pausedUntil` only while a live run waits.
pub fn run_json(run: &AiHandoverBulkRun, requested_by: Option<&RequestedBy>, now: i64) -> Value {
    let paused_until = run
        .paused_until
        .filter(|&t| AI_HANDOVER_BULK_LIVE_STATUSES.contains(&run.status.as_str()) && t > now);
    json!({
        "id": id(run.id),
        "action": run.action,
        "status": run.status,
        "message": run.message,
        "requestedAt": date(run.requested_at),
        "startedAt": run.started_at.map(date),
        "finishedAt": run.finished_at.map(date),
        "processedCount": run.processed_count,
        "skippedCount": run.skipped_count,
        "failedCount": run.failed_count,
        "totalCount": run.total_count,
        "currentError": run.current_error,
        "requestedByName": requested_by.map(|u| u.name.clone().unwrap_or_else(|| u.email.clone())),
        "pausedUntil": paused_until.map(date),
    })
}

/// `toApplyToAllStatus`: `idle` (never switched), `reconciling` (a stored
/// change without its run yet), else the run's status.
pub fn apply_to_all_status_json(
    state: &ApplyToAllStatus,
    is_automation_active: bool,
    now: i64,
) -> Value {
    let status = match &state.run {
        Some(run) => run.status.as_str(),
        None if state.revision > 0 => "reconciling",
        None => "idle",
    };
    json!({
        "status": status,
        "applyToAllCustomers": state.apply_to_all_customers,
        "isAutomationActive": is_automation_active,
        "run": state.run.as_ref().map(|run| run_json(run, None, now)),
    })
}

/// `aiHandoverTimeRangesSchema` issues (`None` = invalid).
pub fn time_ranges_field(input: &mut Input, required: bool) -> Option<Value> {
    let path = key("timeRanges");
    let value = match input.get("timeRanges").cloned() {
        None => {
            if required {
                input.issue(
                    &path,
                    "invalid_type",
                    "Invalid input: expected array, received undefined",
                );
            }
            return None;
        }
        Some(value) => value,
    };
    let Value::Array(items) = &value else {
        input.issue(&path, "invalid_type", "Invalid input: expected array");
        return None;
    };
    let mut ok = true;
    let mut out = Vec::new();
    for (i, item) in items.iter().enumerate() {
        let mut at = path.clone();
        at.push(Seg::Index(i));
        let Value::Object(range) = item else {
            input.issue(&at, "invalid_type", "Invalid input: expected object");
            ok = false;
            continue;
        };
        let from = input.number_at(
            &child(&at, "from"),
            range.get("from"),
            Num::int(0.0, 24.0),
            true,
        );
        let to = input.number_at(
            &child(&at, "to"),
            range.get("to"),
            Num::int(0.0, 24.0),
            true,
        );
        match (from, to) {
            (Some(from), Some(to)) if from != to => {
                out.push(json!({ "from": from as u32, "to": to as u32 }));
            }
            (Some(_), Some(_)) => {
                input.issue(&at, "custom", "Invalid input");
                ok = false;
            }
            _ => ok = false,
        }
    }
    if items.len() > 24 {
        input.issue(
            &path,
            "too_big",
            "Too big: expected array to have <=24 items",
        );
        ok = false;
    }
    ok.then_some(Value::Array(out))
}

/// A `.nullable()` field: `Some(None)` = `null`; `partial` makes it optional.
pub fn nullable_id(input: &mut Input, name: &str, partial: bool) -> Option<Option<i64>> {
    match input.get(name).cloned() {
        None if partial => None,
        Some(Value::Null) => Some(None),
        value => input.id_at(&key(name), value.as_ref(), true).map(Some),
    }
}

fn nullable_message(input: &mut Input, partial: bool) -> Option<Option<String>> {
    let spec = Str {
        max: Some(MESSAGE_MAX_LENGTH),
        ..Str::default()
    };
    match input.get("returnMessage").cloned() {
        None if partial => None,
        Some(Value::Null) => Some(None),
        value => input
            .string_at(&key("returnMessage"), value.as_ref(), spec, true)
            .map(Some),
    }
}

fn bool_field(input: &mut Input, name: &str, partial: bool) -> Option<bool> {
    if partial {
        input.opt_bool(name)
    } else {
        input.bool(name)
    }
}

/// `saveAiHandoverSettingsPublicRequest` / `patchAiHandoverSettingsPublicRequest`.
pub fn parse_settings(input: &mut Input, partial: bool) -> SettingsPatch {
    let patch = SettingsPatch {
        enabled: bool_field(input, "enabled", partial),
        schedule_enabled: bool_field(input, "scheduleEnabled", partial),
        time_ranges: time_ranges_field(input, !partial),
        goto_flow_id: nullable_id(input, "gotoFlowId", partial),
        return_message: nullable_message(input, partial),
        pause_bot_waiting_for_staff: bool_field(input, "pauseBotWaitingForStaff", partial),
    };
    let no_ranges = patch
        .time_ranges
        .as_ref()
        .is_some_and(|r| r.as_array().is_some_and(Vec::is_empty));
    if !partial && !input.failed() && patch.schedule_enabled == Some(true) && no_ranges {
        input.issue(
            &key("timeRanges"),
            "custom",
            "A schedule needs at least one time range",
        );
    }
    patch
}

/// `setApplyToAllPublicRequest`: `(apply, message, dryRun, confirmCount)`.
fn parse_set_apply_to_all(input: &mut Input) -> (bool, Option<String>, bool, Option<i64>) {
    let apply = input.bool("applyToAllCustomers");
    let message = input.opt_string(
        "message",
        Str {
            max: Some(MESSAGE_MAX_LENGTH),
            ..Str::default()
        },
    );
    let dry_run = input.opt_bool("dryRun");
    let confirm = input.opt_number(
        "confirmCount",
        Num {
            int: true,
            min: Some(0.0),
            max: None,
        },
    );
    if !input.failed() {
        let apply = apply.unwrap_or_default();
        if !apply && chatbotx_core::js::trim(message.as_deref().unwrap_or("")).is_empty() {
            input.issue(
                &key("message"),
                "custom",
                "message is required to take customers back",
            );
        }
        if dry_run != Some(true) && confirm.is_none() {
            input.issue(
                &key("confirmCount"),
                "custom",
                "confirmCount is required unless dryRun is true",
            );
        }
    }
    (
        apply.unwrap_or_default(),
        message,
        dry_run == Some(true),
        confirm.map(|n| n as i64),
    )
}

/// `saveAiHandoverSettingsRequest` (the builder form: `returnMessage` is a
/// string, "" = none; `.refine` issue without a message).
pub fn parse_private_settings(input: &mut Input) -> SettingsPatch {
    let patch = SettingsPatch {
        enabled: input.bool("enabled"),
        schedule_enabled: input.bool("scheduleEnabled"),
        time_ranges: time_ranges_field(input, true),
        goto_flow_id: nullable_id(input, "gotoFlowId", false),
        return_message: input
            .string("returnMessage", Str::max(MESSAGE_MAX_LENGTH))
            .map(Some),
        pause_bot_waiting_for_staff: input.bool("pauseBotWaitingForStaff"),
    };
    let no_ranges = patch
        .time_ranges
        .as_ref()
        .and_then(Value::as_array)
        .is_some_and(Vec::is_empty);
    if !input.failed() && patch.schedule_enabled == Some(true) && no_ranges {
        input.issue(&key("timeRanges"), "custom", "Invalid input");
    }
    patch
}

/// `setApplyToAllRequest` (the builder form): `(apply, message)`.
pub fn parse_private_set_apply_to_all(input: &mut Input) -> (bool, String) {
    let apply = input.bool("applyToAllCustomers");
    let message = input.string("message", Str::max(MESSAGE_MAX_LENGTH));
    if !input.failed()
        && apply == Some(false)
        && chatbotx_core::js::trim(message.as_deref().unwrap_or("")).is_empty()
    {
        input.issue(&key("message"), "custom", "Invalid input");
    }
    (apply.unwrap_or_default(), message.unwrap_or_default())
}

fn change_json(change: &service::ApplyToAllChange) -> Value {
    json!({
        "dryRun": false,
        "isChanged": change.is_changed,
        "eligibleCount": null,
        "run": change.run.as_ref().map(|run| run_json(run, None, now_ms())),
    })
}

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    let db = &state.db;
    let ws = call.workspace_id;
    let jobs = state.jobs.as_ref();
    let mut input = match route_id {
        "aiHandover.getSettings" | "aiHandover.listHistory" => {
            Input::from_query(call.params, call.query)
        }
        _ => Input::from_body(call.params, call.body),
    };
    let inbox = input.id("inboxId");
    match route_id {
        "aiHandover.getSettings" => {
            input.finish()?;
            let inbox = inbox.unwrap_or_default();
            let now = now_ms();
            let (settings, status, active) = tx::read(db, move |conn| {
                service::require_inbox(conn, ws, inbox)?;
                Ok((
                    service::find_in(conn, ws, inbox)?,
                    service::find_status_in(conn, ws, inbox)?,
                    service::find_active_in(conn, ws, inbox, now)?.is_some(),
                ))
            })
            .await
            .map_err(|e| business_error(e, FINDING))?;
            let mut body = settings_json(settings.as_ref());
            body["applyToAll"] = apply_to_all_status_json(&status, active, now);
            Ok(json(200, body))
        }
        "aiHandover.saveSettings" | "aiHandover.patchSettings" => {
            let partial = route_id == "aiHandover.patchSettings";
            let patch = parse_settings(&mut input, partial);
            input.finish()?;
            let inbox = inbox.unwrap_or_default();
            let saved = if partial {
                service::patch_settings(db, ws, inbox, patch).await
            } else {
                service::save_settings(
                    db,
                    SaveSettingsInput {
                        workspace_id: ws,
                        inbox_id: inbox,
                        enabled: patch.enabled.unwrap_or_default(),
                        schedule_enabled: patch.schedule_enabled.unwrap_or_default(),
                        time_ranges: patch.time_ranges.unwrap_or_else(|| json!([])),
                        goto_flow_id: patch.goto_flow_id.flatten(),
                        return_message: patch.return_message.flatten(),
                        pause_bot_waiting_for_staff: patch
                            .pause_bot_waiting_for_staff
                            .unwrap_or_default(),
                    },
                )
                .await
            }
            .map_err(|e| business_error(e, APPLYING))?;
            Ok(json(200, settings_json(Some(&saved))))
        }
        "aiHandover.setApplyToAll" => {
            let (apply, message, dry_run, confirm) = parse_set_apply_to_all(&mut input);
            input.finish()?;
            let inbox = inbox.unwrap_or_default();
            if dry_run {
                let preview = service::preview_apply_to_all(db, ws, inbox, apply, message)
                    .await
                    .map_err(|e| business_error(e, APPLYING))?;
                return Ok(json(
                    200,
                    json!({
                        "dryRun": true,
                        "run": null,
                        "isChanged": preview.is_changed,
                        "eligibleCount": preview.eligible_count,
                    }),
                ));
            }
            let change = service::set_apply_to_all(
                db,
                jobs,
                SetApplyToAllInput {
                    workspace_id: ws,
                    inbox_id: inbox,
                    user_id: None,
                    apply_to_all_customers: apply,
                    message,
                    confirm_max_eligible: confirm,
                },
            )
            .await
            .map_err(|e| business_error(e, APPLYING))?;
            Ok(json(200, change_json(&change)))
        }
        "aiHandover.retryApplyToAll" => {
            input.finish()?;
            let change = service::retry(db, jobs, ws, inbox.unwrap_or_default(), None)
                .await
                .map_err(|e| business_error(e, APPLYING))?;
            Ok(json(200, change_json(&change)))
        }
        _ => {
            let (page, per_page) = public_paging(&mut input);
            input.finish()?;
            let inbox = inbox.unwrap_or_default();
            let (rows, page_count) = tx::read(db, move |conn| {
                service::require_inbox(conn, ws, inbox)?;
                service::list_history_in(
                    conn,
                    ws,
                    inbox,
                    PaginationInput {
                        page: Some(page),
                        per_page: Some(per_page),
                    },
                )
            })
            .await
            .map_err(|e| business_error(e, FINDING))?;
            let now = now_ms();
            let data: Vec<Value> = rows
                .iter()
                .map(|row| run_json(&row.run, row.requested_by.as_ref(), now))
                .collect();
            Ok(json(200, json!({ "data": data, "pageCount": page_count })))
        }
    }
}
