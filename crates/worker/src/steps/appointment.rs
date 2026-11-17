//! `appointmentScheduling` (`integration/handlers/appointment-scheduling.ts`).

use chatbotx_business::BusinessError;
use chatbotx_business::appointment::booking::{self, BookInput, Deps as BookingDeps};
use chatbotx_business::appointment::external::ExternalCalendars;
use chatbotx_business::appointment::{booking::scheduling_copy, calendar};
use chatbotx_business::contact_custom_field::{self, SetValueByKeyInput};
use chatbotx_business::tx;
use chatbotx_core::encryption::tokens::{
    APPOINTMENT_WEBVIEW_TOKEN_TTL_MS, AppointmentTokenSource, AppointmentWebviewClaims,
    AppointmentWebviewMode,
};
use chatbotx_db::repo::now_ms;
use chatbotx_db::rusqlite::{OptionalExtension as _, params};
use chatbotx_jobs::JobError;
use serde_json::{Value, json};

use super::{StepCtx, step_error};
use crate::flow::StepStatus;
use crate::{db_fail, id_of};

const WEBVIEW_SELECTION: &str = "appointmentWebviewSelection";
const RANGE_SELECTION: &str = "appointmentAvailabilityRangeSelection";
const RANGE_SKIPPED: &str = "appointmentAvailabilityRangeSkipped";

pub(crate) async fn run(
    ctx: &StepCtx<'_, '_>,
    step_type: &str,
) -> Result<Option<StepStatus>, JobError> {
    if step_type != "appointmentScheduling" {
        return Ok(None);
    }
    let mode = ctx.str("mode").unwrap_or("").to_owned();
    match handle(ctx, &mode).await {
        Ok(status) => Ok(Some(status)),
        Err(error) => {
            // The original catches every failure: an error result, no retry.
            tracing::error!(
                workspace_id = ctx.workspace_id(),
                conversation_id = ctx.run.conversation.id,
                contact_id = ctx.contact_id(),
                calendar_id = ctx.str("calendarId"),
                mode = %mode,
                err = %error,
                "Appointment scheduling step failed"
            );
            Ok(Some(StepStatus::Error))
        }
    }
}

/// The metadata of the run when it is the `kind` payload of this step.
fn step_metadata<'a>(ctx: &'a StepCtx<'_, '_>, kind: &str) -> Option<&'a Value> {
    ctx.run.metadata().filter(|m| {
        m.get("type").and_then(Value::as_str) == Some(kind)
            && m.get("stepId").and_then(Value::as_str) == ctx.str("id")
    })
}

async fn handle(ctx: &StepCtx<'_, '_>, mode: &str) -> Result<StepStatus, String> {
    let run = ctx.run;
    let ws = ctx.workspace_id();
    let contact_id = ctx.contact_id();
    let calendar_id = ctx
        .step
        .get("calendarId")
        .and_then(id_of)
        .ok_or("calendarId missing")?;
    let external = ExternalCalendars {
        encryption: crate::channels::meta_webhook::encryption_from_env(),
        ..Default::default()
    };
    let deps = BookingDeps {
        db: &run.deps.db,
        jobs: Some(run.deps.jobs.as_ref()),
        external: &external,
    };
    let copy = scheduling_copy(run.contact_inbox.language.as_deref());
    let err = |e: BusinessError| e.to_string();
    match mode {
        "bookFromCustomField" => {
            let cal = tx::read(&run.deps.db, move |conn| {
                calendar::find_or_fail(conn, ws, calendar_id)
            })
            .await
            .map_err(err)?;
            if let Some(max) = cal.max_appointments_per_user {
                let held = tx::read(&run.deps.db, move |conn| {
                    Ok(
                        chatbotx_db::repo::appointment::list_future_scheduled_for_contact(
                            conn,
                            ws,
                            calendar_id,
                            contact_id,
                            now_ms(),
                        )?
                        .len() as i64,
                    )
                })
                .await
                .map_err(err)?;
                if held >= max {
                    return Ok(step_error(ctx, "appointment_already_scheduled"));
                }
            }
            let field = ctx.step.get("dateTimeFieldId").and_then(id_of);
            let value = match field {
                Some(field) => find_value(ctx, field).await.map_err(|e| e.to_string())?,
                None => None,
            };
            let Some(start_at) = value.as_deref().and_then(chatbotx_core::js::parse_date) else {
                return Ok(step_error(ctx, "invalidDateTimeFieldValue"));
            };
            book(ctx, deps, calendar_id, start_at).await
        }
        "checkAvailabilityFromCustomField" => {
            let start = match ctx.step.get("startDateFieldId").and_then(id_of) {
                Some(f) => find_value(ctx, f).await.map_err(|e| e.to_string())?,
                None => None,
            };
            let end = match ctx.step.get("endDateFieldId").and_then(id_of) {
                Some(f) => find_value(ctx, f).await.map_err(|e| e.to_string())?,
                None => None,
            };
            let parse = |v: Option<String>| v.as_deref().and_then(chatbotx_core::js::parse_date);
            match (parse(start), parse(end)) {
                (Some(start), Some(end)) if start <= end => {
                    availability(ctx, deps, calendar_id, start, end, copy.no_slots_found).await
                }
                _ => Ok(step_error(ctx, "invalidAvailabilityRange")),
            }
        }
        "book" => {
            if step_metadata(ctx, WEBVIEW_SELECTION).is_some() {
                return Ok(StepStatus::Success);
            }
            send_webview(
                ctx,
                calendar_id,
                AppointmentWebviewMode::Book,
                "/booking/picker",
                copy.booking_prompt,
                copy.booking_button,
            )
            .await?;
            Ok(StepStatus::Wait)
        }
        "cancel" => {
            booking::cancel_for_contact(
                deps,
                ws,
                calendar_id,
                contact_id,
                Some(run.conversation.id),
                Some(run.contact_inbox.id),
                run.metadata().cloned(),
            )
            .await
            .map_err(err)?;
            Ok(StepStatus::Success)
        }
        "checkAvailability" => {
            if step_metadata(ctx, RANGE_SKIPPED).is_some() {
                tracing::warn!(
                    step_id = ctx.str("id"),
                    reason = "range_skipped",
                    "Appointment scheduling availability range skipped"
                );
                return Ok(step_error(ctx, "range_skipped"));
            }
            let Some(selection) = step_metadata(ctx, RANGE_SELECTION) else {
                if ctx.str("outputCustomFieldId").is_none_or(str::is_empty) {
                    return Ok(step_error(ctx, "missingOutputCustomFieldId"));
                }
                send_webview(
                    ctx,
                    calendar_id,
                    AppointmentWebviewMode::SelectAvailabilityRange,
                    "/booking/range-picker",
                    copy.range_prompt,
                    copy.range_button,
                )
                .await?;
                return Ok(StepStatus::Wait);
            };
            let cal = tx::read(&run.deps.db, move |conn| {
                calendar::find_or_fail(conn, ws, calendar_id)
            })
            .await
            .map_err(err)?;
            let local = |key: &str| {
                selection
                    .get(key)
                    .and_then(Value::as_str)
                    .and_then(|t| chatbotx_core::utils::tz::from_zoned_time(t, &cal.timezone).ok())
            };
            match (local("startDate"), local("endDate")) {
                (Some(start), Some(end)) if start <= end => {
                    availability(ctx, deps, calendar_id, start, end, copy.no_slots_found).await
                }
                _ => Ok(StepStatus::Error),
            }
        }
        _ => Ok(StepStatus::Error),
    }
}

/// `contactCustomFieldService.findValue`.
async fn find_value(ctx: &StepCtx<'_, '_>, field_id: i64) -> Result<Option<String>, JobError> {
    let contact_id = ctx.contact_id();
    ctx.run
        .deps
        .db
        .read(move |conn| {
            Ok(conn
                .query_row(
                    r#"SELECT "value" FROM "ContactCustomField" WHERE "contactId" = ?1 AND "customFieldId" = ?2"#,
                    params![contact_id, field_id],
                    |row| row.get::<_, Option<String>>(0),
                )
                .optional()?
                .flatten())
        })
        .await
        .map_err(db_fail)
}

/// `bookAppointmentAndMapErrors`.
async fn book(
    ctx: &StepCtx<'_, '_>,
    deps: BookingDeps<'_>,
    calendar_id: i64,
    start_at: i64,
) -> Result<StepStatus, String> {
    let run = ctx.run;
    let result = booking::book(
        deps,
        BookInput {
            workspace_id: ctx.workspace_id(),
            calendar_id,
            contact_id: ctx.contact_id(),
            conversation_id: Some(run.conversation.id),
            contact_inbox_id: Some(run.contact_inbox.id),
            start_at,
            invitee_timezone: None,
            metadata: run.metadata().cloned(),
        },
    )
    .await;
    match result {
        Ok(_) => Ok(StepStatus::Success),
        Err(BusinessError::Exception { code, .. }) => match code.as_str() {
            "slotUnavailable" => Ok(step_error(ctx, "slot_unavailable")),
            "appointmentAvailabilityChanged" => Ok(step_error(ctx, "availability_changed")),
            "appointmentAlreadyScheduled" => Ok(step_error(ctx, "appointment_already_scheduled")),
            other => Err(other.to_string()),
        },
        Err(error) => Err(error.to_string()),
    }
}

/// `checkAvailability` + the output custom field write.
/// ponytail: `resultUsedByAI` keeps the raw availability text (no AI
/// rewrite); add the AI candidates loop when that step option is needed.
async fn availability(
    ctx: &StepCtx<'_, '_>,
    deps: BookingDeps<'_>,
    calendar_id: i64,
    start: i64,
    end: i64,
    no_slots: &str,
) -> Result<StepStatus, String> {
    let (text, slots, _) = booking::check_availability(
        deps,
        ctx.workspace_id(),
        calendar_id,
        Some(ctx.contact_id()),
        start,
        end,
    )
    .await
    .map_err(|e| e.to_string())?;
    let text = if slots.is_empty() {
        no_slots.to_string()
    } else {
        text
    };
    if let Some(field) = ctx.str("outputCustomFieldId").filter(|f| !f.is_empty()) {
        let input = SetValueByKeyInput {
            workspace_id: ctx.workspace_id(),
            contact_id: ctx.contact_id(),
            keyword: field.to_owned(),
            value: text,
            ..SetValueByKeyInput::default()
        };
        contact_custom_field::set_value_by_key(&ctx.run.deps.db, input)
            .await
            .map_err(|e| e.message().to_string())?;
    }
    Ok(StepStatus::Success)
}

/// Signs the webview token and sends the button message (`sendChatMessage`).
async fn send_webview(
    ctx: &StepCtx<'_, '_>,
    calendar_id: i64,
    mode: AppointmentWebviewMode,
    path: &str,
    text: &str,
    label: &str,
) -> Result<(), String> {
    let run = ctx.run;
    let encryption = crate::channels::meta_webhook::encryption_from_env()
        .ok_or("ENCRYPTION_KEY is not configured")?;
    let range = matches!(mode, AppointmentWebviewMode::SelectAvailabilityRange);
    let claims = AppointmentWebviewClaims {
        source: AppointmentTokenSource::Flow,
        mode,
        workspace_id: ctx.workspace_id().to_string(),
        calendar_id: calendar_id.to_string(),
        contact_id: ctx.contact_id().to_string(),
        conversation_id: run.conversation.id.to_string(),
        contact_inbox_id: run.contact_inbox.id.to_string(),
        channel: Some(run.contact_inbox.channel.clone()),
        flow_id: Some(run.flow_version.flow_id.to_string()),
        flow_version_id: Some(run.flow_version.id.to_string()),
        step_id: ctx.str("id").map(str::to_owned),
        node_id: Some(run.target_node_id.clone()),
        result_custom_field_id: range
            .then(|| ctx.str("outputCustomFieldId").map(str::to_owned))
            .flatten(),
        result_used_by_ai: range
            .then(|| ctx.step.get("resultUsedByAI").and_then(Value::as_bool))
            .flatten(),
        ..Default::default()
    };
    let token = encryption
        .sign_appointment_webview_token(&claims, APPOINTMENT_WEBVIEW_TOKEN_TTL_MS)
        .map_err(|e| e.to_string())?;
    let mut url = url::Url::parse(&run.deps.app_url)
        .and_then(|base| base.join(path))
        .map_err(|e| e.to_string())?;
    url.query_pairs_mut().append_pair("token", &token);
    let mut data = serde_json::Map::new();
    data.insert(
        "conversationId".into(),
        json!(run.conversation.id.to_string()),
    );
    data.insert(
        "contactInboxId".into(),
        json!(run.contact_inbox.id.to_string()),
    );
    data.insert("text".into(), json!(text));
    data.insert(
        "quickReplies".into(),
        json!([{
            "id": chatbotx_core::utils::id::create_id(),
            "label": label,
            "buttonType": "url",
            "url": url.to_string(),
            "messengerExtensions": true,
        }]),
    );
    for key in ["trackingContext", "metadata"] {
        if let Some(value) = run.data.get(key).filter(|v| !v.is_null()) {
            data.insert(key.into(), value.clone());
        }
    }
    if let Some(bulk) = run.is_bulk_broadcast {
        data.insert("isBulkBroadcast".into(), json!(bulk));
    }
    run.deps
        .jobs
        .add(
            chatbotx_jobs::QueueName::Chat,
            chatbotx_jobs::chat_wait::SEND_CHAT_MESSAGE_ACTION,
            json!({ "type": chatbotx_jobs::chat_wait::SEND_CHAT_MESSAGE_ACTION, "data": Value::Object(data) }),
            chatbotx_jobs::JobOptions::default(),
        )
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}
